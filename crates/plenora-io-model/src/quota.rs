//! Catalogo autoritativo delle quote di risorse. La deadline produce timeout.
use crate::budget::PipelineLimits;
use crate::{
    ErrorCategory, ErrorPhase, IoErrorCode, PlenoraIoError, PublicMessage, RemoteEffect,
    RetryDisposition,
};

#[derive(Clone, Copy, Debug)]
pub struct QuotaRule {
    pub quota: Quota,
    pub name: &'static str,
    pub message: &'static str,
    pub cli: Option<&'static str>,
    pub runtime: &'static str,
    pub sdk: Option<&'static str>,
    pub default: u64,
    pub unit: &'static str,
    pub scope: &'static str,
    pub phases: &'static [ErrorPhase],
}
macro_rules! quotas {
    ($(($id:ident, $field:ident, $setter:ident, $default:expr, $unit:literal, $scope:literal, [$($phase:ident),+], $flag:literal)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Quota { $($id),+, ConcurrentOperations }
        pub const QUOTAS: &[QuotaRule] = &[$(Quota::$id.rule()),+, Quota::ConcurrentOperations.rule()];
        impl Quota {
            #[must_use]
            pub const fn rule(self) -> QuotaRule {
                match self {
                    $(Self::$id => QuotaRule { quota: self, name: stringify!($field), message: concat!(stringify!($field), " quota superata"), cli: Some($flag),
                        runtime: concat!("PipelineLimits.", stringify!($field)), sdk: Some(stringify!($field)),
                        default: $default, unit: $unit, scope: $scope, phases: &[$(ErrorPhase::$phase),+] }),+,
                    Self::ConcurrentOperations => QuotaRule { quota: self, name: "concurrent_operations", message: "concurrent_operations quota superata", cli: None,
                        runtime: "ResourcePoolBuilder.concurrent_operations", sdk: None, default: 64,
                        unit: "operazioni", scope: "pool condiviso", phases: &[ErrorPhase::Prepare, ErrorPhase::Read, ErrorPhase::Write] },
                }
            }
            #[must_use]
            pub fn value(self, limits: &PipelineLimits) -> Option<u64> {
                match self { $(Self::$id => u64::try_from(limits.$field()).ok()),+, Self::ConcurrentOperations => None }
            }
            /// Applica un intero positivo alla configurazione, senza perdere overflow.
            /// # Errors
            /// Rifiuta valori fuori dominio; validate verifica le relazioni fra quote.
            pub fn set(self, limits: PipelineLimits, value: u64) -> crate::Result<PipelineLimits> {
                if value == 0 { return Err(self.invalid()); }
                match self { $(Self::$id => Ok(limits.$setter(value.try_into().map_err(|_| self.invalid())?))),+,
                    Self::ConcurrentOperations => Err(self.invalid()) }
            }
        }
    };
}
quotas! {
    (InputBytes, max_input_bytes, with_max_input_bytes, 268_435_456, "byte", "totale sorgente e spool dei decoder", [Read], "--max-input-bytes"),
    (InputEntries, max_input_entries, with_max_input_entries, 10_000, "entry", "totale sorgente", [Read], "--max-input-entries"),
    (Rows, max_rows, with_max_rows, 10_000_000, "righe", "totale operazione", [Read, Write], "--max-rows"),
    (Columns, max_columns, with_max_columns, 4_096, "colonne", "schema layer e totale del piano di scrittura", [Validate, Prepare, Read, Write], "--max-columns"),
    (GeometryComponents, max_geometry_components, with_max_geometry_components, 16_777_216, "componenti", "totale operazione", [Read, Write], "--max-geometry-components"),
    (OutputBytes, max_output_bytes, with_max_output_bytes, 1_073_741_824, "byte", "totale operazione", [Read, Write, Finalize, Commit], "--max-output-bytes"),
    (OutputExpansionRatio, output_expansion_ratio, with_output_expansion_ratio, 1_073_741_824, "moltiplicatore", "uscita / ingresso osservato non vuoto", [Read, Write, Finalize, Commit], "--output-expansion-ratio"),
    (WkbCellBytes, max_wkb_cell_bytes, with_max_wkb_cell_bytes, 67_108_864, "byte", "per geometria", [Read, Write], "--max-wkb-cell-bytes"),
    (WkbComponents, max_wkb_components, with_max_wkb_components, 100_000, "componenti", "per geometria", [Read, Write], "--max-wkb-components"),
    (WkbDepth, max_wkb_depth, with_max_wkb_depth, 32, "livelli", "per geometria", [Read, Write], "--max-wkb-depth"),
    (Vertices, max_vertices, with_max_vertices, 50_000_000, "componenti (coordinate e membri)", "per geometria; alias storico che stringe max_wkb_components", [Read, Write], "--max-vertices"),
    (MemoryBytes, memory_bytes, with_memory_bytes, 536_870_912, "byte", "memoria viva pipeline e pool", [Prepare, Read, Write, Finalize], "--memory-bytes"),
    (SpillBytes, spill_bytes, with_spill_bytes, 4_294_967_296, "byte", "spill vivo pipeline e pool", [Read, Write, Finalize], "--spill-bytes"),
    (DecompressionRatio, decompression_ratio, with_decompression_ratio, 1_000, "moltiplicatore", "totale archivio compresso", [Read], "--decompression-ratio"),
}
impl Quota {
    #[must_use]
    pub fn exceeded(self, phase: ErrorPhase) -> PlenoraIoError {
        PlenoraIoError::redatto(
            IoErrorCode::LimitExceeded,
            ErrorCategory::ResourceLimit,
            phase,
            RemoteEffect::None,
            RetryDisposition::Never,
            &PublicMessage::Curated(self.rule().message),
        )
    }
    /// Verifica un rapporto senza overflow e senza allargare il dominio di configurazione.
    /// # Errors
    /// Quota superata se il numeratore supera il prodotto saturato.
    pub fn check_ratio(
        self,
        expanded: u64,
        compressed: u64,
        maximum: u64,
        phase: ErrorPhase,
    ) -> crate::Result<()> {
        if maximum == 0 {
            return Err(self.invalid());
        }
        if expanded > 0 && (compressed == 0 || expanded > compressed.saturating_mul(maximum)) {
            return Err(self.exceeded(phase));
        }
        Ok(())
    }
    #[must_use]
    pub fn exceeded_measured(self, phase: ErrorPhase, observed: u64, limit: u64) -> PlenoraIoError {
        let mut error = PlenoraIoError::redatto(
            IoErrorCode::LimitExceeded,
            ErrorCategory::ResourceLimit,
            phase,
            RemoteEffect::None,
            RetryDisposition::Never,
            &PublicMessage::CuratedBetween(
                self.rule().name,
                crate::NumeroStrutturale::Conteggio(observed),
                "oltre il limite di",
                crate::NumeroStrutturale::Limite(limit),
            ),
        );
        error.quota_details = Some(Box::new(crate::error::QuotaExceeded {
            quota: self.rule().name.to_owned(),
            observed,
            limit,
        }));
        error
    }
    #[must_use]
    pub fn invalid(self) -> PlenoraIoError {
        crate::error_table::Situation::InvalidQuota.error_with(&PublicMessage::CuratedPair(
            self.rule().name,
            "deve essere un intero positivo e coerente con le altre quote",
        ))
    }
    /// # Errors
    /// Zero, segno negativo, testo e overflow sono configurazione non valida.
    pub fn parse(self, text: &str) -> crate::Result<u64> {
        text.parse::<u64>()
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| self.invalid())
    }
}

impl PipelineLimits {
    #[must_use]
    pub fn quota_error(&self, mut error: PlenoraIoError, phase: ErrorPhase) -> PlenoraIoError {
        if error.category == ErrorCategory::ResourceLimit {
            if self.max_vertices() < self.max_wkb_components()
                && error.message == Quota::WkbComponents.exceeded(phase).message
            {
                error.message = Quota::Vertices.exceeded(phase).message;
            }
            error.phase = phase;
        }
        error
    }
}

#[cfg(test)]
mod tests;
