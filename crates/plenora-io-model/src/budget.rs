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
mod tests;
