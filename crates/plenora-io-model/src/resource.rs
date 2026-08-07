//! Budget condiviso di risorse per una pipeline I/O (R7.5-R7.7).
//!
//! I clone condividono gli stessi contatori e la stessa deadline. Una lease
//! prenota quota prima dell'allocazione; al drop la restituisce, mentre
//! `commit` trasforma la parte usata in consumo cumulativo.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{PlenoraIoError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    MemoryBytes,
    Rows,
    Columns,
    GeometryComponents,
    ConcurrentOperations,
    OutputBytes,
    SpillBytes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLimits {
    pub memory_bytes: u64,
    pub rows: u64,
    pub columns: u64,
    pub geometry_components: u64,
    pub nesting_depth: u64,
    pub concurrent_operations: u64,
    pub output_bytes: u64,
    pub output_expansion_ratio: u64,
    pub duration_ms: u64,
    pub spill_bytes: u64,
    pub cell_bytes: u64,
    pub decompression_ratio: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 512 * 1024 * 1024,
            rows: u64::MAX,
            columns: 65_536,
            geometry_components: 16_777_216,
            nesting_depth: 64,
            concurrent_operations: 64,
            output_bytes: u64::MAX,
            output_expansion_ratio: 1_000,
            duration_ms: 30_000,
            spill_bytes: 4 * 1024 * 1024 * 1024,
            cell_bytes: 64 * 1024 * 1024,
            decompression_ratio: 1_000,
        }
    }
}

impl ResourceLimits {
    /// Verifica gli invarianti dei limiti prima di costruire un budget.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se un limite è nullo o se
    /// `cell_bytes` supera il budget di memoria.
    pub fn validate(&self) -> Result<()> {
        if self.memory_bytes == 0
            || self.rows == 0
            || self.columns == 0
            || self.geometry_components == 0
            || self.nesting_depth == 0
            || self.concurrent_operations == 0
            || self.output_bytes == 0
            || self.output_expansion_ratio == 0
            || self.duration_ms == 0
            || self.spill_bytes == 0
            || self.cell_bytes == 0
            || self.decompression_ratio == 0
        {
            return Err(limit("i limiti di risorsa devono essere maggiori di zero"));
        }
        if self.cell_bytes > self.memory_bytes {
            return Err(limit("cell_bytes supera il budget di memoria"));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Counters {
    memory_bytes: AtomicU64,
    rows: AtomicU64,
    columns: AtomicU64,
    geometry_components: AtomicU64,
    concurrent_operations: AtomicU64,
    output_bytes: AtomicU64,
    spill_bytes: AtomicU64,
    input_bytes: AtomicU64,
}

impl Counters {
    const fn new(limits: &ResourceLimits) -> Self {
        Self {
            memory_bytes: AtomicU64::new(limits.memory_bytes),
            rows: AtomicU64::new(limits.rows),
            columns: AtomicU64::new(limits.columns),
            geometry_components: AtomicU64::new(limits.geometry_components),
            concurrent_operations: AtomicU64::new(limits.concurrent_operations),
            output_bytes: AtomicU64::new(limits.output_bytes),
            spill_bytes: AtomicU64::new(limits.spill_bytes),
            input_bytes: AtomicU64::new(0),
        }
    }

    const fn get(&self, kind: ResourceKind) -> &AtomicU64 {
        match kind {
            ResourceKind::MemoryBytes => &self.memory_bytes,
            ResourceKind::Rows => &self.rows,
            ResourceKind::Columns => &self.columns,
            ResourceKind::GeometryComponents => &self.geometry_components,
            ResourceKind::ConcurrentOperations => &self.concurrent_operations,
            ResourceKind::OutputBytes => &self.output_bytes,
            ResourceKind::SpillBytes => &self.spill_bytes,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResourceBudget {
    limits: Arc<ResourceLimits>,
    counters: Arc<Counters>,
    deadline: Instant,
}

impl ResourceBudget {
    /// Costruisce un budget condiviso con scadenza assoluta.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se i limiti non sono
    /// validi o se la deadline non è rappresentabile da [`Instant`].
    pub fn new(limits: ResourceLimits) -> Result<Self> {
        limits.validate()?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(limits.duration_ms))
            .ok_or_else(|| limit("deadline risorse oltre Instant"))?;
        let counters = Counters::new(&limits);
        Ok(Self {
            limits: Arc::new(limits),
            counters: Arc::new(counters),
            deadline,
        })
    }

    #[must_use]
    pub fn limits(&self) -> &ResourceLimits {
        &self.limits
    }

    #[must_use]
    pub fn remaining(&self, kind: ResourceKind) -> u64 {
        self.counters.get(kind).load(Ordering::Acquire)
    }

    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    #[must_use]
    pub fn remaining_duration(&self) -> Option<Duration> {
        self.deadline.checked_duration_since(Instant::now())
    }

    /// Verifica che la finestra temporale del budget non sia esaurita.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se la deadline è passata.
    pub fn ensure_active(&self) -> Result<()> {
        if self.remaining_duration().is_none() {
            Err(limit("durata del budget di risorse esaurita"))
        } else {
            Ok(())
        }
    }

    #[must_use]
    pub fn is_same_budget(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.counters, &other.counters)
    }

    /// Preleva una quota dal budget restituendo la lease corrispondente.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se `amount` è zero, se la
    /// deadline è scaduta o se la quota residua non è sufficiente.
    pub fn try_lease(&self, kind: ResourceKind, amount: u64) -> Result<ResourceLease> {
        if amount == 0 {
            return Err(limit("una lease di risorsa deve essere maggiore di zero"));
        }
        self.ensure_active()?;
        let counter = self.counters.get(kind);
        let mut current = counter.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_sub(amount) else {
                return Err(limit("budget di risorsa esaurito"));
            };
            match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    return Ok(ResourceLease {
                        budget: self.clone(),
                        kind,
                        amount,
                        released: false,
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    /// Registra i byte fisici di input una volta risolta la sorgente. Il dato
    /// alimenta il limite di espansione dell'output e attraversa i clone.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se la deadline è scaduta
    /// o se il conteggio dei byte di input andrebbe in overflow.
    pub fn observe_input_bytes(&self, amount: u64) -> Result<()> {
        self.ensure_active()?;
        let mut current = self.counters.input_bytes.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(amount) else {
                return Err(limit("overflow nel conteggio dei byte di input"));
            };
            match self.counters.input_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
    }

    #[must_use]
    pub fn observed_input_bytes(&self) -> u64 {
        self.counters.input_bytes.load(Ordering::Acquire)
    }

    /// Tetto assoluto derivato sia dalla quota output sia dal fattore rispetto
    /// all'input. In una scrittura standalone senza input osservato resta il
    /// solo limite assoluto.
    #[must_use]
    pub fn output_limit(&self) -> u64 {
        let input = self.observed_input_bytes();
        if input == 0 {
            return self.limits.output_bytes;
        }
        let expanded = input.saturating_mul(self.limits.output_expansion_ratio);
        self.limits.output_bytes.min(expanded)
    }

    fn release(&self, kind: ResourceKind, amount: u64) -> Result<()> {
        let counter = self.counters.get(kind);
        let maximum = maximum_for(&self.limits, kind);
        let mut current = counter.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(amount) else {
                return Err(limit("overflow durante la restituzione del budget"));
            };
            if next > maximum {
                return Err(limit("restituzione superiore alla quota originaria"));
            }
            match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
    }
}

impl Default for ResourceBudget {
    fn default() -> Self {
        let limits = ResourceLimits::default();
        let deadline = Instant::now() + Duration::from_millis(limits.duration_ms);
        let counters = Counters::new(&limits);
        Self {
            limits: Arc::new(limits),
            counters: Arc::new(counters),
            deadline,
        }
    }
}

const fn maximum_for(limits: &ResourceLimits, kind: ResourceKind) -> u64 {
    match kind {
        ResourceKind::MemoryBytes => limits.memory_bytes,
        ResourceKind::Rows => limits.rows,
        ResourceKind::Columns => limits.columns,
        ResourceKind::GeometryComponents => limits.geometry_components,
        ResourceKind::ConcurrentOperations => limits.concurrent_operations,
        ResourceKind::OutputBytes => limits.output_bytes,
        ResourceKind::SpillBytes => limits.spill_bytes,
    }
}

#[derive(Debug)]
pub struct ResourceLease {
    budget: ResourceBudget,
    kind: ResourceKind,
    amount: u64,
    released: bool,
}

impl ResourceLease {
    #[must_use]
    pub const fn amount(&self) -> u64 {
        self.amount
    }

    /// Consuma `used` dalla lease e restituisce al budget la parte inutilizzata.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se `used` è zero o supera
    /// la lease, oppure se la restituzione della quota residua fallisce.
    pub fn commit(mut self, used: u64) -> Result<()> {
        if used == 0 || used > self.amount {
            return Err(limit("consumo non valido per la lease di risorsa"));
        }
        let unused = self.amount - used;
        if unused > 0 {
            self.budget.release(self.kind, unused)?;
        }
        self.released = true;
        Ok(())
    }

    /// Restituisce al budget l'intera quota prelevata.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::LimitExceeded`] se la restituzione
    /// andrebbe in overflow o supererebbe la quota originaria.
    pub fn release(mut self) -> Result<()> {
        self.budget.release(self.kind, self.amount)?;
        self.released = true;
        Ok(())
    }
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        if !self.released && self.budget.release(self.kind, self.amount).is_ok() {
            self.released = true;
        }
    }
}

fn limit(message: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::LimitExceeded(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(memory_bytes: u64) -> ResourceBudget {
        ResourceBudget::new(ResourceLimits {
            memory_bytes,
            cell_bytes: memory_bytes,
            ..ResourceLimits::default()
        })
        .unwrap()
    }

    #[test]
    fn leases_cross_clones_and_return_quota_on_drop() {
        let budget = budget(100);
        let clone = budget.clone();
        let lease = clone.try_lease(ResourceKind::MemoryBytes, 60).unwrap();
        assert_eq!(budget.remaining(ResourceKind::MemoryBytes), 40);
        assert!(budget.try_lease(ResourceKind::MemoryBytes, 41).is_err());
        drop(lease);
        assert_eq!(budget.remaining(ResourceKind::MemoryBytes), 100);
    }

    #[test]
    fn checked_commit_returns_only_unused_quota() {
        let budget = budget(100);
        budget
            .try_lease(ResourceKind::MemoryBytes, 80)
            .unwrap()
            .commit(30)
            .unwrap();
        assert_eq!(budget.remaining(ResourceKind::MemoryBytes), 70);
        assert!(budget.try_lease(ResourceKind::MemoryBytes, 0).is_err());
    }

    #[test]
    fn budget_identity_is_explicit() {
        let first = budget(100);
        assert!(first.is_same_budget(&first.clone()));
        assert!(!first.is_same_budget(&budget(100)));
    }

    #[test]
    fn output_limit_uses_shared_observed_input() {
        let budget = ResourceBudget::new(ResourceLimits {
            output_bytes: 1_000,
            output_expansion_ratio: 3,
            ..ResourceLimits::default()
        })
        .unwrap();
        budget.observe_input_bytes(100).unwrap();
        // Il clone e' il soggetto dell'asserzione: verifica che l'input
        // osservato sia condiviso fra i cloni, non un dettaglio ridondante.
        #[allow(clippy::redundant_clone)]
        let observed_by_clone = budget.clone().output_limit();
        assert_eq!(observed_by_clone, 300);
    }

    #[test]
    fn deadline_expires_for_every_clone() {
        let budget = ResourceBudget::new(ResourceLimits {
            duration_ms: 1,
            ..ResourceLimits::default()
        })
        .unwrap();
        let clone = budget.clone();
        std::thread::sleep(Duration::from_millis(5));
        assert!(budget.ensure_active().is_err());
        assert!(clone.try_lease(ResourceKind::Rows, 1).is_err());
    }
}
