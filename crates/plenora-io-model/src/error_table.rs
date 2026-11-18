//! Classificazione comune delle situazioni di errore del bordo I/O.
use crate::{
    ErrorCategory, ErrorPhase, IoErrorCode, PlenoraIoError, PublicMessage, RemoteEffect,
    RetryDisposition,
};

#[derive(Clone, Copy, Debug)]
pub struct ErrorRule {
    pub situation: Situation,
    pub category: ErrorCategory,
    pub phase: ErrorPhase,
    pub code: IoErrorCode,
    pub wire_code: &'static str,
    pub message: &'static str,
}

macro_rules! rules {
    ($(($s:ident, $c:ident, $p:ident, $i:ident, $w:literal, $m:literal)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Situation { $($s),+ }
        pub const ERROR_RULES: &[ErrorRule] = &[$(Situation::$s.rule()),+];
        impl Situation {
            #[must_use]
            pub const fn rule(self) -> ErrorRule {
                match self { $(Self::$s => ErrorRule {
                    situation: self, category: ErrorCategory::$c,
                    phase: ErrorPhase::$p, code: IoErrorCode::$i, wire_code: $w, message: $m,
                }),+ }
            }
        }
    };
}

rules! {
    (MissingSource, NotFound, Read, Io, "SOURCE_NOT_FOUND", "sorgente o membro obbligatorio del dataset non trovato"),
    (InvalidContent, DataMapping, Read, Format, "FORMAT_ERROR", "contenuto del dataset vuoto o non valido"),
    (DirectoryInsteadOfFile, InvalidConfiguration, Validate, Generic, "CLI_USAGE", "la sorgente deve essere un file, non una directory"),
    (InvalidConfiguration, InvalidConfiguration, Validate, Generic, "CLI_USAGE", "configurazione non valida"),
    (MissingOptionColumn, InvalidConfiguration, Validate, Generic, "CLI_USAGE", "colonna indicata nelle opzioni non presente"),
    (ContradictedCrs, Crs, Validate, Generic, "CRS_CONFLICT", "il CRS assunto contraddice quello dichiarato"),
    (MissingGeometry, Unsupported, Write, Capability, "UNSUPPORTED", "il formato di destinazione richiede una geometria"),
    (UnrepresentableWriteType, Unsupported, Write, Capability, "UNSUPPORTED", "tipo non rappresentabile nel formato di destinazione"),
    (NonFiniteWriteValue, DataMapping, Write, Format, "FORMAT_ERROR", "valore non finito non rappresentabile in scrittura"),
    (MissingLayer, NotFound, Prepare, Generic, "NO_LAYER", "layer richiesto non presente"),
    (InvalidQuota, InvalidConfiguration, Validate, Generic, "CLI_USAGE", "le quote devono essere positive e coerenti"),
    (InvalidSpillDirectory, InvalidConfiguration, Validate, Generic, "CLI_USAGE", "PLENORA_SPILL_DIR non indica una directory utilizzabile"),
    (MissingWriteBackend, Unsupported, Write, Unsupported, "UNSUPPORTED", "scrittura FileGDB non disponibile: compilare con --features gdal-backend"),
}

impl Situation {
    #[must_use]
    pub fn error(self) -> PlenoraIoError {
        self.error_with(&PublicMessage::Curated(self.rule().message))
    }

    #[must_use]
    pub fn error_with(self, message: &PublicMessage) -> PlenoraIoError {
        let rule = self.rule();
        PlenoraIoError::redatto(
            rule.code,
            rule.category,
            rule.phase,
            RemoteEffect::None,
            RetryDisposition::Never,
            message,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_unique_and_construct_the_declared_error() {
        for (index, rule) in ERROR_RULES.iter().enumerate() {
            assert!(!ERROR_RULES[..index]
                .iter()
                .any(|other| other.situation == rule.situation));
            let error = rule.situation.error();
            assert_eq!(
                (error.category, error.phase, error.code),
                (rule.category, rule.phase, rule.code)
            );
            assert!(!error.message.contains("  "));
        }
    }
}
