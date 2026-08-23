//! Modello budget unificato (`PRODUCT.md § Budget e limiti`,
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
//! use plenora_io_model::budget::{
//!     ObservedInput, PipelineBudget, PipelineLimits, SourceEntry,
//! };
//!
//! // convert: un solo bundle, due rami con contatori indipendenti.
//! let bundle = PipelineBudget::builder()
//!     .limits(PipelineLimits::default().with_max_rows(1_000))
//!     .build()?;
//! let (read, write) = bundle.into_convert_parts().into_parts();
//!
//! // il preflight del core enumera la sorgente e poi consuma il permit:
//! // byte, entry e digest sono tutti accumulati qui, non dichiarati.
//! // `into_components` e' l'unica via che separa il permit dalle parti, ed
//! // e' workspace-internal: fuori da model/core il gate la rifiuta.
//! let (read_budget, permit, _atteso) = read.into_components();
//! let permit = permit.ok_or("permit assente")?;
//! let context = read_budget.context();
//! context.note_entry_visited(&SourceEntry::directory(b"dati", None))?;
//! context.note_entry_visited(&SourceEntry::file(b"dati/a.csv", 4_096, None))?;
//! context.observe_input(permit)?;
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
//! let parts = PipelineBudget::builder().build().expect("costruito").into_read_parts();
//! let (_budget, permit, _atteso) = parts.into_components();
//! let permit = permit.expect("permit presente");
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::cancellation::CancellationToken;
use crate::error::{ErrorPhase, PlenoraIoError, PublicMessage, Result};

/// Identita' di pipeline, usata per legare un [`InputPermit`] al
/// [`PipelineContext`] che lo ha emesso. Un contatore monotono e non un
/// confronto di puntatori: l'indirizzo di un `Arc` liberato puo' essere
/// riusato, un id no.
static NEXT_PIPELINE_ID: AtomicU64 = AtomicU64::new(1);

/// Alloca un'identita' di pipeline senza wrap.
///
/// `fetch_add` avvolge in silenzio: all'esaurimento dello spazio degli id
/// due pipeline distinte riceverebbero lo stesso valore, e il permit
/// dell'una diventerebbe spendibile sull'altra. E' il contrario esatto di
/// cio' che l'identita' serve a garantire, quindi qui si fallisce chiuso.
///
/// L'ultimo id non viene mai consegnato: il contatore conserva il
/// *prossimo* valore, e rifiutarsi di superarlo tiene l'invariante
/// "contatore sempre incrementabile" senza casi limite.
fn allocate_pipeline_id(counter: &AtomicU64) -> Result<u64> {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        let Some(next) = current.checked_add(1) else {
            return Err(limit_error(PIPELINE_IDS_EXHAUSTED));
        };
        #[cfg(test)]
        perdi_la_corsa(counter);
        match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Ok(current),
            Err(observed) => current = observed,
        }
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        // Un lock avvelenato significa che un thread e' andato in panico
        // mentre teneva lo stato. Lo stato resta comunque coerente: ogni
        // transizione qui e' un'assegnazione singola dopo tutti i controlli,
        // quindi non esiste il mezzo aggiornamento che il poisoning teme.
        Err(poisoned) => poisoned.into_inner(),
    }
}

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
const TOO_MANY_INPUT_BYTES: &str = "byte di input oltre il limite";
const INPUT_BYTES_OVERFLOW: &str = "overflow nel conteggio dei byte di input";
const PIPELINE_IDS_EXHAUSTED: &str = "spazio delle identita' di pipeline esaurito";
const SHRINK_ABOVE_RESERVATION: &str = "la riduzione supera la quota gia' prenotata";
const SHRINK_TO_ZERO: &str = "un batch custodito non puo' occupare zero byte";

/// Una quota superata.
///
/// Il parametro era gia' `&'static str` prima di S9 — nessuno di questi
/// messaggi e' mai stato costruito a runtime — quindi la migrazione qui e' un
/// cambio di costruttore e nient'altro: stesso testo, stesso wire, ma ora il
/// tipo dice che non puo' essere altrimenti.
fn limit_error(message: &'static str) -> PlenoraIoError {
    PlenoraIoError::limite_redatto(&PublicMessage::Curated(message))
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

/// Natura dell'entry. Entra nel digest come tag esplicito: senza, un file
/// vuoto e una directory sullo stesso path avrebbero la stessa codifica.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKind {
    File,
    Directory,
}

impl EntryKind {
    const fn tag(self) -> u8 {
        match self {
            Self::File => 1,
            Self::Directory => 2,
        }
    }
}

/// Identita' di una entry osservata dal preflight.
///
/// Il path arriva **gia' normalizzato** dal core: la normalizzazione dipende
/// dal filesystem e dalla piattaforma, che il modello non conosce.
///
/// I due valori in byte sono distinti e non intercambiabili:
///
/// - `metadata_size` entra nel **digest**: e' cio' che rende rilevabile una
///   mutazione in place;
/// - `charged_input_bytes` conta verso **`max_input_bytes`**: e' cio' che il
///   bordo si impegna a leggere davvero.
///
/// Per una directory il secondo e' zero — non c'e' contenuto da leggere — e
/// anche il primo lo e': la dimensione riportata di una directory e' un
/// artefatto del filesystem, e le sue voci sono gia' nel digest una per una,
/// quindi includerla aggiungerebbe rumore senza aggiungere rilevazione. Le
/// due costruzioni sono metodi distinti proprio perche' la regola resti
/// strutturale invece che una convenzione del chiamante.
#[derive(Clone, Copy, Debug)]
pub struct SourceEntry<'a> {
    path_identity_bytes: &'a [u8],
    kind: EntryKind,
    metadata_size: u64,
    charged_input_bytes: u64,
    modified: Option<SystemTime>,
}

impl<'a> SourceEntry<'a> {
    /// Entry di file: la dimensione entra nel digest **e** viene addebitata
    /// a `max_input_bytes`.
    #[must_use]
    pub const fn file(
        path_identity_bytes: &'a [u8],
        size_bytes: u64,
        modified: Option<SystemTime>,
    ) -> Self {
        Self {
            path_identity_bytes,
            kind: EntryKind::File,
            metadata_size: size_bytes,
            charged_input_bytes: size_bytes,
            modified,
        }
    }

    /// Entry di directory: conta per `max_input_entries`, non addebita byte
    /// e non porta dimensione nel digest.
    #[must_use]
    pub const fn directory(path_identity_bytes: &'a [u8], modified: Option<SystemTime>) -> Self {
        Self {
            path_identity_bytes,
            kind: EntryKind::Directory,
            metadata_size: 0,
            charged_input_bytes: 0,
            modified,
        }
    }

    /// Codifica **senza perdita** del percorso lessicale, cosi' come il
    /// chiamante lo ha visto.
    ///
    /// Non e' un percorso normalizzato e non e' un'identita' canonica del
    /// filesystem: `a/../b` e `b` restano distinti, e due hard link allo
    /// stesso inode pure. Lo dice il nome perche' il nome precedente,
    /// `normalized_path`, prometteva una normalizzazione che nessuno faceva.
    ///
    /// La canonicalizzazione e' **deliberatamente esclusa**:
    /// `std::fs::canonicalize` segue i symlink, e il preflight li rifiuta
    /// invece di seguirli. Farla qui allargherebbe il contratto della
    /// sorgente proprio nel punto in cui e' stato ristretto.
    ///
    /// Serve una sola proprieta': **iniettivita' e stabilita'**. Due corse
    /// sulla stessa sorgente devono dare lo stesso digest, e due sorgenti
    /// diverse no. Una conversione con perdita — `to_string_lossy` sostituisce
    /// ogni sequenza non valida con U+FFFD — romperebbe la seconda meta'.
    #[must_use]
    pub const fn path_identity_bytes(&self) -> &'a [u8] {
        self.path_identity_bytes
    }

    #[must_use]
    pub const fn metadata_size(&self) -> u64 {
        self.metadata_size
    }

    #[must_use]
    pub const fn charged_input_bytes(&self) -> u64 {
        self.charged_input_bytes
    }

    #[must_use]
    pub const fn is_directory(&self) -> bool {
        matches!(self.kind, EntryKind::Directory)
    }

    #[must_use]
    pub const fn modified(&self) -> Option<SystemTime> {
        self.modified
    }

    /// Codifica canonica dell'entry: lunghezza del path, path, tipo,
    /// dimensione di metadata e mtime con segno esplicito. La lunghezza in
    /// testa impedisce che due insiemi di path diversi producano la stessa
    /// sequenza di byte.
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(self.path_identity_bytes.len() + 34);
        encoded.extend_from_slice(&(self.path_identity_bytes.len() as u64).to_le_bytes());
        encoded.extend_from_slice(self.path_identity_bytes);
        encoded.push(self.kind.tag());
        encoded.extend_from_slice(&self.metadata_size.to_le_bytes());
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
///
/// Non usa atomiche: vive dentro lo stato di osservazione, protetto dallo
/// stesso mutex che rende conteggio, byte e digest un aggiornamento unico.
#[derive(Clone, Copy, Debug, Default)]
struct DigestAccumulator {
    high: u64,
    low: u64,
}

impl DigestAccumulator {
    fn absorb(&mut self, entry: &SourceEntry<'_>) {
        let encoded = entry.canonical_bytes();
        self.high ^= fnv1a(FNV_BASIS_HIGH, &encoded);
        self.low ^= fnv1a(FNV_BASIS_LOW, &encoded);
    }

    const fn finish(self) -> SourceDigest {
        let high = self.high.to_le_bytes();
        let low = self.low.to_le_bytes();
        SourceDigest([
            high[0], high[1], high[2], high[3], high[4], high[5], high[6], high[7], low[0], low[1],
            low[2], low[3], low[4], low[5], low[6], low[7],
        ])
    }
}

/// Digest opaco a 128 bit sull'insieme delle entry osservate: path
/// normalizzati, tipo, dimensione di metadata e mtime.
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

// Interferenza concorrente simulata, per le sole sonde di questo modulo.
//
// Il ramo `Err` di `compare_exchange_weak` si esegue solo quando lo scambio
// perde la corsa. Se venga eseguito o no dipende dallo scheduling, e la
// copertura di quelle righe cambiava fra due misure sullo **stesso albero**:
// e' la causa dimostrata del blocco `copertura.variazione-fra-corse`.
//
// Qui la corsa si perde **su richiesta**. Il valore viene mutato appena prima
// dello scambio, che fallisce per la ragione vera — il valore osservato non e'
// piu' quello atteso — e non per un fallimento simulato: il ramo eseguito e'
// quello di produzione, non una sua imitazione.
//
// L'alternativa sarebbe sostituire `compare_exchange_weak` con la variante
// forte. Sarebbe una modifica dell'algoritmo motivata dalla misura, e la
// variante debole e' li' per una ragione che la misura non conosce.
//
// Le due direzioni servono entrambe: i gauge scendono quando qualcuno preleva,
// il contatore degli identificatori sale quando qualcuno ne alloca uno.
#[cfg(test)]
#[derive(Clone, Copy)]
enum InterferenzaConcorrente {
    Sottrae(u64),
    Aggiunge(u64),
}

#[cfg(test)]
thread_local! {
    static INTERFERENZA: std::cell::Cell<Option<InterferenzaConcorrente>> =
        const { std::cell::Cell::new(None) };
}

/// Arma **una sola** interferenza sul prossimo scambio di questo thread.
#[cfg(test)]
fn arma_interferenza(quale: InterferenzaConcorrente) {
    INTERFERENZA.with(|cella| cella.set(Some(quale)));
}

/// Consuma l'armamento, se c'e', mutando il valore prima dello scambio.
#[cfg(test)]
fn perdi_la_corsa(valore: &AtomicU64) {
    match INTERFERENZA.with(std::cell::Cell::take) {
        Some(InterferenzaConcorrente::Sottrae(quanto)) => {
            valore.fetch_sub(quanto, Ordering::AcqRel);
        }
        Some(InterferenzaConcorrente::Aggiunge(quanto)) => {
            valore.fetch_add(quanto, Ordering::AcqRel);
        }
        None => {}
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
            #[cfg(test)]
            perdi_la_corsa(&self.remaining);
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

    /// Preleva `amount` rispettando **insieme** la capacita' residua e un
    /// tetto derivato sul consumo cumulativo, in una sola osservazione
    /// atomica.
    ///
    /// Calcolare il consumo proiettato con una `load` e poi prelevare con una
    /// `compare_exchange` separata lascia una finestra fra le due: due
    /// richieste concorrenti possono osservare lo stesso consumo, superare
    /// entrambe il controllo del tetto e prelevare entrambe. Il tetto
    /// verrebbe cosi' sforato senza che nessuna delle due lo veda. Qui
    /// proiezione e prelievo avvengono sullo **stesso** valore osservato:
    /// se il CAS fallisce, il tetto viene riproiettato sul valore nuovo.
    fn try_take_bounded(&self, amount: u64, ceiling: u64) -> TakeOutcome {
        let mut current = self.remaining.load(Ordering::Acquire);
        loop {
            let consumed = self.capacity.saturating_sub(current);
            let Some(projected) = consumed.checked_add(amount) else {
                return TakeOutcome::AboveCeiling;
            };
            if projected > ceiling {
                return TakeOutcome::AboveCeiling;
            }
            let Some(next) = current.checked_sub(amount) else {
                return TakeOutcome::Exhausted;
            };
            #[cfg(test)]
            perdi_la_corsa(&self.remaining);
            match self.remaining.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return TakeOutcome::Taken,
                Err(observed) => current = observed,
            }
        }
    }

    fn give_back(&self, amount: u64) {
        let mut current = self.remaining.load(Ordering::Acquire);
        loop {
            let next = current.saturating_add(amount).min(self.capacity);
            #[cfg(test)]
            perdi_la_corsa(&self.remaining);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TakeOutcome {
    Taken,
    Exhausted,
    AboveCeiling,
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

    /// Limiti WKB effettivi per singola geometria, nella forma che i driver
    /// passano al decoder.
    ///
    /// E' il sostituto di `Limits::effective_wkb()`: stessi tre valori, con
    /// il tetto dei componenti gia' composto con `max_vertices`.
    #[must_use]
    pub fn wkb_limits(&self) -> crate::limits::WkbLimits {
        crate::limits::WkbLimits {
            max_cell_bytes: self.max_wkb_cell_bytes,
            max_components: self.effective_wkb_components(),
            max_depth: self.max_wkb_depth,
        }
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

/// Osservazione della sorgente come **state machine linearizzabile**.
///
/// Conteggio delle entry, byte addebitati e digest non sono tre contatori
/// indipendenti: sono tre facce dello stesso fatto, "quali entry ho
/// osservato". Tenerli in atomiche separate avrebbe reso osservabile uno
/// stato intermedio — entry gia' contata e byte non ancora sommati, o
/// viceversa — e avrebbe reso possibile un aggiornamento parziale in caso di
/// errore. Sotto un mutex unico la transizione e' una sola assegnazione dopo
/// tutti i controlli: o passa tutta o non passa niente.
///
/// La pubblicazione e' terminale: dopo `Published` nessuna entry nuova e'
/// accettabile, perche' il footprint gia' consegnato dichiara un insieme che
/// non puo' piu' cambiare.
#[derive(Debug)]
enum SourceObservation {
    Collecting {
        entries: u64,
        total_bytes: u64,
        digest: DigestAccumulator,
    },
    Published(SourceFootprint),
}

impl SourceObservation {
    const fn new() -> Self {
        Self::Collecting {
            entries: 0,
            total_bytes: 0,
            digest: DigestAccumulator { high: 0, low: 0 },
        }
    }

    const fn observed_input(&self) -> ObservedInput {
        match self {
            Self::Collecting { .. } => ObservedInput::NotObserved,
            Self::Published(footprint) => ObservedInput::Bytes(footprint.total_bytes),
        }
    }

    const fn entries(&self) -> u64 {
        match self {
            Self::Collecting { entries, .. } => *entries,
            Self::Published(footprint) => footprint.entries_visited,
        }
    }

    const fn charged_bytes(&self) -> u64 {
        match self {
            Self::Collecting { total_bytes, .. } => *total_bytes,
            Self::Published(footprint) => footprint.total_bytes,
        }
    }

    /// Accetta una entry aggiornando conteggio, byte e digest in un atto
    /// unico. Tutti i controlli precedono ogni scrittura: un rifiuto non
    /// lascia nulla di aggiornato.
    fn accept(&mut self, entry: &SourceEntry<'_>, limits: &PipelineLimits) -> Result<()> {
        let Self::Collecting {
            entries,
            total_bytes,
            digest,
        } = self
        else {
            return Err(limit_error(INPUT_ALREADY_OBSERVED));
        };

        let next_entries = entries
            .checked_add(1)
            .ok_or_else(|| limit_error(ENTRIES_OVERFLOW))?;
        if next_entries > limits.max_input_entries {
            return Err(limit_error(TOO_MANY_ENTRIES));
        }
        let next_bytes = total_bytes
            .checked_add(entry.charged_input_bytes)
            .ok_or_else(|| limit_error(INPUT_BYTES_OVERFLOW))?;
        if next_bytes > limits.max_input_bytes {
            return Err(limit_error(TOO_MANY_INPUT_BYTES));
        }

        *entries = next_entries;
        *total_bytes = next_bytes;
        digest.absorb(entry);
        Ok(())
    }

    /// Transizione terminale: sigilla l'insieme osservato in un footprint.
    fn publish(&mut self) -> Result<SourceFootprint> {
        let Self::Collecting {
            entries,
            total_bytes,
            digest,
        } = self
        else {
            return Err(limit_error(INPUT_ALREADY_OBSERVED));
        };
        let footprint = SourceFootprint {
            total_bytes: *total_bytes,
            entries_visited: *entries,
            digest: digest.finish(),
        };
        *self = Self::Published(footprint);
        Ok(footprint)
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
    observation: Mutex<SourceObservation>,
    memory: Gauge,
    spill: Gauge,
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
        lock_recover(&self.inner.observation).observed_input()
    }

    /// Entry osservate finora, o quelle del footprint dopo la pubblicazione.
    #[must_use]
    pub fn entries_visited(&self) -> u64 {
        lock_recover(&self.inner.observation).entries()
    }

    /// Byte addebitati a `max_input_bytes` finora. Le directory non ne
    /// addebitano, quindi questo valore non e' il numero di entry per la
    /// dimensione media: e' cio' che il bordo si impegna a leggere.
    #[must_use]
    pub fn charged_input_bytes(&self) -> u64 {
        lock_recover(&self.inner.observation).charged_bytes()
    }

    /// Residuo **locale** di memoria, senza guardare il pool.
    ///
    /// Serve a chi vuole sapere quanto resta a questa pipeline in isolamento.
    /// Chi deve decidere quanto prenotare usi
    /// [`Self::effective_remaining_memory`]: `lease_memory_internal` compone
    /// locale e pool (INV-12), quindi il solo residuo locale sovrastima cio'
    /// che entrerebbe davvero.
    #[must_use]
    pub fn remaining_memory(&self) -> u64 {
        self.inner.memory.remaining()
    }

    /// Residuo locale di spill, senza guardare il pool. Vedi
    /// [`Self::remaining_memory`].
    #[must_use]
    pub fn remaining_spill(&self) -> u64 {
        self.inner.spill.remaining()
    }

    /// Residuo di memoria **effettivo**: il minimo fra locale e pool.
    ///
    /// E' il numero che governa cio' che una prenotazione puo' ottenere. Con
    /// quota locale ampia e pool stretto, dimensionare sul solo residuo
    /// locale porta a chiedere piu' di quanto entri: la lease fallisce, e il
    /// chiamante interpreta come "memoria esaurita" cio' che era soltanto una
    /// richiesta mal dimensionata — invece di prenotare il possibile e
    /// migrare su disco.
    #[must_use]
    pub fn effective_remaining_memory(&self) -> u64 {
        self.effective_remaining(GaugeKind::Memory)
    }

    /// Residuo di spill effettivo, con la stessa composizione.
    #[must_use]
    pub fn effective_remaining_spill(&self) -> u64 {
        self.effective_remaining(GaugeKind::Spill)
    }

    /// Capacita' di memoria effettiva: il minimo fra locale e pool.
    ///
    /// E' la grandezza da cui derivare una soglia, non
    /// `PipelineLimits::memory_bytes`: con un pool piu' stretto, una soglia
    /// calcolata sul solo limite locale sarebbe irraggiungibile, e lo spool
    /// non migrerebbe mai — cioe' resterebbe inutile proprio nel caso in cui
    /// serve.
    #[must_use]
    pub fn effective_memory_capacity(&self) -> u64 {
        self.effective_capacity(GaugeKind::Memory)
    }

    /// Capacita' di spill effettiva, con la stessa composizione.
    #[must_use]
    pub fn effective_spill_capacity(&self) -> u64 {
        self.effective_capacity(GaugeKind::Spill)
    }

    fn effective_remaining(&self, kind: GaugeKind) -> u64 {
        let locale = kind.local(&self.inner).remaining();
        self.inner
            .pool
            .as_ref()
            .map_or(locale, |pool| locale.min(kind.pooled(pool).remaining()))
    }

    fn effective_capacity(&self, kind: GaugeKind) -> u64 {
        let locale = kind.local(&self.inner).capacity();
        self.inner
            .pool
            .as_ref()
            .map_or(locale, |pool| locale.min(kind.pooled(pool).capacity()))
    }

    /// Osserva l'input consumando il `permit` e pubblica il footprint.
    ///
    /// Unica fabbrica di [`SourceFootprint`] e unico canale di
    /// registrazione (INV-13). One-shot per costruzione: il permit e' preso
    /// per `move` e non e' `Clone`, quindi una seconda osservazione con lo
    /// stesso permit non e' scrivibile.
    ///
    /// **Non ha parametri oltre al permit.** Byte, entry e digest sono tutti
    /// accumulati dal context durante l'enumerazione via
    /// [`Self::note_entry_visited`]: il footprint pubblicato descrive
    /// esattamente cio' che il preflight ha osservato, e non c'e' alcun
    /// valore che il chiamante possa dichiarare senza averlo misurato. Con i
    /// byte come parametro sarebbe rimasta una seconda sorgente di verita'
    /// per la grandezza che governa `output_expansion_ratio`.
    ///
    /// La pubblicazione e' **terminale**: dopo questa chiamata ogni nuova
    /// entry viene rifiutata, perche' il footprint consegnato dichiara un
    /// insieme che non puo' piu' cambiare.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se il permit appartiene
    /// a un'altra pipeline o se l'input risulta gia' pubblicato, e l'errore
    /// di [`Self::ensure_active`] se la pipeline non e' piu' attiva.
    ///
    /// **Un errore non modifica lo stato precedente.** Detto altrimenti: la
    /// chiamata o pubblica, o non lascia traccia. Non equivale a dire che lo
    /// stato resti `Collecting` — formulazione che questa doc portava fino a
    /// S4.b.3 ed era falsa nel caso del secondo publish: li' lo stato
    /// precedente e' `Published`, l'errore lo lascia `Published`, e
    /// [`ObservedInput`] continua a riportare il footprint gia' registrato.
    /// La pubblicazione e' terminale in entrambe le direzioni: non si
    /// ripubblica, e non si torna indietro.
    // Il passaggio per valore e' l'invariante, non una svista: il permit e'
    // one-shot e non `Clone`, quindi consumarlo qui e' cio' che rende
    // impossibile una seconda osservazione. Prenderlo per riferimento — o
    // renderlo `Copy`, come suggerisce il lint — riaprirebbe esattamente il
    // buco che INV-13 chiude.
    #[allow(clippy::needless_pass_by_value)]
    pub fn observe_input(&self, permit: InputPermit) -> Result<SourceFootprint> {
        // Destrutturare consuma il permit qui, non al termine dello scope:
        // dopo questa riga non esiste piu' un valore spendibile altrove.
        let InputPermit { pipeline_id } = permit;
        if pipeline_id != self.inner.pipeline_id {
            return Err(limit_error(PERMIT_FOREIGN));
        }
        self.ensure_active()?;
        lock_recover(&self.inner.observation).publish()
    }

    /// Registra una entry visitata durante l'enumerazione della sorgente.
    ///
    /// E' l'unico punto in cui la sorgente viene osservata, e applica in un
    /// solo atto le tre grandezze che descrivono l'insieme osservato:
    /// `max_input_entries` (INV-9), `max_input_bytes` sui byte addebitati
    /// dall'entry, e il digest dell'identita'. Sono tre facce dello stesso
    /// fatto, non tre contatori indipendenti: separarle avrebbe reso
    /// osservabile uno stato intermedio e possibile un aggiornamento
    /// parziale.
    ///
    /// I controlli precedono ogni scrittura, quindi **un rifiuto non lascia
    /// nulla di aggiornato**: ne' il conteggio, ne' i byte, ne' il digest.
    ///
    /// Una directory conta per il numero di entry ma addebita zero byte: e'
    /// proprio la parte che `max_input_bytes` da solo non vedrebbe.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se l'input e' gia'
    /// pubblicato, se l'entry supererebbe `max_input_entries` o
    /// `max_input_bytes`, o se uno dei due conteggi andrebbe in overflow; e
    /// l'errore di [`Self::ensure_active`] se la pipeline non e' attiva.
    pub fn note_entry_visited(&self, entry: &SourceEntry<'_>) -> Result<()> {
        self.ensure_active()?;
        lock_recover(&self.inner.observation).accept(entry, &self.inner.limits)
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
        let pipeline_id = allocate_pipeline_id(&NEXT_PIPELINE_ID)?;
        let context = PipelineContext {
            inner: Arc::new(ContextInner {
                pipeline_id,
                deadline,
                cancellation: self.cancellation,
                limits,
                observation: Mutex::new(SourceObservation::new()),
                memory: Gauge::new(limits.memory_bytes),
                spill: Gauge::new(limits.spill_bytes),
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
        let gauge = self.counters.get(counter);
        // Il tetto derivato vincola solo l'output: per gli altri contatori
        // coincide con la capacita', quindi non e' mai il vincolo che lega.
        // Il tetto si legge una volta sola ed e' corretto farlo: deriva
        // dall'osservazione dell'input, che e' one-shot e viene pubblicata
        // dal preflight prima che esista un `OperationBudget` da cui
        // prelevare output.
        let ceiling = if counter == OperationCounter::OutputBytes {
            self.output_limit()
        } else {
            gauge.capacity()
        };
        match gauge.try_take_bounded(amount, ceiling) {
            TakeOutcome::Taken => Ok(CountedLease {
                budget: self.clone(),
                counter,
                amount,
                released: false,
                not_sync: PhantomData,
            }),
            TakeOutcome::AboveCeiling if counter == OperationCounter::OutputBytes => {
                Err(limit_error(OUTPUT_LIMIT_EXCEEDED))
            }
            TakeOutcome::AboveCeiling | TakeOutcome::Exhausted => {
                Err(limit_error(COUNTER_EXHAUSTED))
            }
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

    /// Riduce la prenotazione a `bytes`, restituendo solo l'eccedenza.
    ///
    /// E' l'handoff della memoria fra chi materializza un batch e chi lo
    /// custodisce. Il materializzatore prenota largo — target del batch piu'
    /// il tetto per cella — perche' prima di leggere non sa quanto occupera'
    /// davvero; a batch costruito la grandezza e' nota, e la prenotazione va
    /// portata a quella.
    ///
    /// Deve avvenire **senza restituire e riprendere**. Rilasciare la lease e
    /// riacquistarne una piu' piccola lascerebbe un istante in cui il batch e'
    /// in RAM e non lo conta nessuno: con un budget condiviso — cioe'
    /// `convert` — un'altra operazione puo' infilarsi in quella finestra e
    /// prenotare memoria che di fatto non c'e'. Qui la quota contabilizzata
    /// scende da `self.bytes` a `bytes` e basta: non passa mai per zero.
    ///
    /// Dopo la riduzione la lease si sposta per `move` a chi custodisce il
    /// batch. Un `move` non tocca il gauge, quindi il passaggio di proprieta'
    /// e' gratuito e per costruzione senza finestra.
    ///
    /// **Ridurre a zero e' rifiutato.** Un batch custodito occupa sempre
    /// almeno il proprio ingombro strutturale — l'elemento in coda, l'`Arc`
    /// dello schema, i metadati Arrow — anche quando non ha righe ne'
    /// colonne. Una lease da zero byte dichiarerebbe che un oggetto vivo non
    /// occupa nulla, cioe' rimetterebbe in circolo la stessa finestra non
    /// contabilizzata che `shrink_to` esiste per chiudere, solo scritta in un
    /// altro modo. Chi vuole davvero smettere di contabilizzare il batch
    /// rilascia la lease, e allora il batch non e' piu' custodito.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se `bytes` e' zero, o se
    /// supera la quota gia' prenotata: questo metodo riduce soltanto.
    /// Ingrandire richiederebbe di prenotare altro, che puo' fallire, e non
    /// sarebbe piu' un handoff ma una seconda prenotazione.
    pub fn shrink_to(&mut self, bytes: u64) -> Result<()> {
        if bytes == 0 {
            return Err(limit_error(SHRINK_TO_ZERO));
        }
        if bytes > self.bytes {
            return Err(limit_error(SHRINK_ABOVE_RESERVATION));
        }
        let eccedenza = self.bytes - bytes;
        if eccedenza > 0 {
            self.bytes = bytes;
            self.context.give_back_shared(eccedenza, GaugeKind::Memory);
        }
        Ok(())
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

    /// Scompone le parti nei componenti trasportati, **per move**.
    ///
    /// # API workspace-internal
    ///
    /// Questo e' l'**unico** punto in cui il permit si separa dalle parti, ed
    /// e' riservato a `plenora-io-model` e `plenora-io-core`. Fino a S4.b.3
    /// esisteva accanto a questo un `take_input_permit()` pubblico: due vie
    /// per la stessa separazione, di cui una contraddiceva la lettera di
    /// INV-13. Ne resta una, marcata e sorvegliata dal gate
    /// `scripts/check_permit_boundary.py`.
    ///
    /// Rust non sa esprimere "pubblico dentro il workspace": `pub(crate)` non
    /// basta — il core e' un crate distinto — e non esiste un `pub(workspace)`.
    /// Il confine e' quindi convenzionale, e regge su tre fatti verificabili
    /// invece che su una promessa del linguaggio: entrambi i crate sono
    /// `publish = false`, l'elemento e' `#[doc(hidden)]`, e il gate rifiuta
    /// qualunque uso fuori dai due crate.
    ///
    /// Consumare le parti, invece di prestarne un riferimento, e' cio' che
    /// rende esplicito che quello trasportato e' l'unico esemplare: clonare
    /// il budget sarebbe innocuo per i contatori — condividono lo stesso
    /// `PipelineContext` — ma renderebbe indistinguibile il passaggio dalla
    /// rigenerazione, e il permit non e' clonabile affatto.
    #[doc(hidden)]
    #[must_use]
    pub fn into_components(
        self,
    ) -> (
        OperationBudget,
        Option<InputPermit>,
        Option<SourceFootprintSnapshot>,
    ) {
        (self.budget, self.permit, self.expected)
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

    /// Estrae il budget **per move**, con la stessa motivazione e lo stesso
    /// confine workspace-internal di [`ReadBudgetParts::into_components`].
    ///
    /// Non trasporta permit — il ramo write non osserva input — quindi qui il
    /// confine protegge solo la distinzione fra move e rigenerazione.
    #[doc(hidden)]
    #[must_use]
    pub fn into_budget(self) -> OperationBudget {
        self.budget
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
}
