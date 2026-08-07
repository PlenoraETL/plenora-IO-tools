use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const ROW_DIAGNOSTICS_CONTRACT: &str = "plenora-row-diagnostics-v1";
pub const ROW_DIAGNOSTICS_INDEX_BASIS: &str = "source_row_zero_based";
pub const ROW_DIAGNOSTIC_COLUMN_UNATTESTABLE: &str = "diagnostic_column_name_unattestable";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowDiagnosticColumn {
    Attested(String),
    Unattestable,
}

impl RowDiagnosticColumn {
    pub fn attest(value: impl Into<String>) -> Self {
        let value = value.into();
        if !value.is_empty() && value.chars().count() <= 256 {
            Self::Attested(value)
        } else {
            Self::Unattestable
        }
    }

    #[must_use]
    pub fn into_option(self) -> Option<String> {
        match self {
            Self::Attested(value) => Some(value),
            Self::Unattestable => None,
        }
    }

    #[must_use]
    pub const fn is_attested(&self) -> bool {
        matches!(self, Self::Attested(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowDiagnosticScope {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowDiagnosticsCompleteness {
    Complete,
    Partial,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowDiagnosticKeyState {
    Value,
    Redacted,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RowDiagnosticKeyValue {
    String(String),
    Integer(i64),
    Boolean(bool),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowDiagnosticKey {
    pub field: String,
    pub state: RowDiagnosticKeyState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<RowDiagnosticKeyValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowDiagnosticWriteState {
    CertainlyRejected,
    CertainlyNotAttempted,
    CertainlyRolledBack,
    EffectUnknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowDiagnosticExample {
    pub source_index: u64,
    pub cause: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<RowDiagnosticKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_state: Option<RowDiagnosticWriteState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum KnownOrUnknownCount {
    Known { value: u64 },
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteDiagnosticStateCounts {
    pub certainly_rejected: u64,
    pub certainly_not_attempted: u64,
    pub certainly_rolled_back: u64,
    pub effect_unknown: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowDiagnosticWriteOutcome {
    pub certainly_rejected: KnownOrUnknownCount,
    pub certainly_not_attempted: KnownOrUnknownCount,
    pub certainly_rolled_back: KnownOrUnknownCount,
    pub effect_unknown: KnownOrUnknownCount,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowDiagnostics {
    pub contract: String,
    pub scope: RowDiagnosticScope,
    pub index_basis: String,
    pub completeness: RowDiagnosticsCompleteness,
    pub knowledge_limits: Option<Vec<String>>,
    pub observed_total: u64,
    pub total: Option<u64>,
    pub input_total: Option<u64>,
    pub counts: BTreeMap<String, u64>,
    pub examples_limit: u64,
    pub examples_truncated: bool,
    pub examples: Vec<RowDiagnosticExample>,
    pub diagnostic_state_counts: Option<WriteDiagnosticStateCounts>,
    pub write_outcome: Option<RowDiagnosticWriteOutcome>,
}

impl RowDiagnostics {
    /// Verifica sia la forma sia gli invarianti aritmetici di
    /// `plenora-row-diagnostics-v1`. La serializzazione richiama sempre questo
    /// metodo, così un documento costruito manualmente non può oltrepassare il
    /// bordo pubblico in uno stato non conforme.
    ///
    /// # Errors
    ///
    /// Restituisce la descrizione del primo invariante violato: contratto o
    /// `index_basis` non riconosciuto, `examples_limit` nullo, conteggi non
    /// coerenti con `observed_total`, completezza incompatibile con i
    /// conteggi, esempi o chiavi non conformi.
    // La sequenza di controlli resta lineare e monolitica per essere
    // confrontabile, nell'ordine, con il contratto normativo
    // `plenora-row-diagnostics-v1`.
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.contract != ROW_DIAGNOSTICS_CONTRACT {
            return Err("contratto row diagnostics non riconosciuto".to_owned());
        }
        if self.index_basis != ROW_DIAGNOSTICS_INDEX_BASIS {
            return Err("index_basis row diagnostics non riconosciuto".to_owned());
        }
        if self.examples_limit == 0 {
            return Err("examples_limit deve essere almeno 1".to_owned());
        }
        let counts_sum = self.counts.iter().try_fold(0_u64, |sum, (cause, count)| {
            if !valid_token(cause) || *count == 0 {
                return Err("causa o conteggio row diagnostics non valido".to_owned());
            }
            sum.checked_add(*count)
                .ok_or_else(|| "overflow nei conteggi row diagnostics".to_owned())
        })?;
        if counts_sum != self.observed_total {
            return Err("counts non somma a observed_total".to_owned());
        }
        match self.completeness {
            RowDiagnosticsCompleteness::Complete => {
                if self.total != Some(self.observed_total) || self.observed_total == 0 {
                    return Err(
                        "report complete richiede total positivo uguale a observed_total"
                            .to_owned(),
                    );
                }
                if self.knowledge_limits.is_some() {
                    return Err("report complete non ammette knowledge_limits".to_owned());
                }
            }
            RowDiagnosticsCompleteness::Partial => {
                validate_knowledge_limits(self.knowledge_limits.as_deref())?;
                if self
                    .total
                    .is_some_and(|total| total < self.observed_total || total == 0)
                {
                    return Err("total parziale non valido".to_owned());
                }
            }
            RowDiagnosticsCompleteness::Unknown => {
                validate_knowledge_limits(self.knowledge_limits.as_deref())?;
                if self.total.is_some() {
                    return Err("report unknown non ammette total".to_owned());
                }
            }
        }
        let examples_len = u64::try_from(self.examples.len())
            .map_err(|_| "troppi esempi row diagnostics".to_owned())?;
        if examples_len > self.examples_limit || examples_len > self.observed_total {
            return Err("examples supera il limite o observed_total".to_owned());
        }
        if self.completeness == RowDiagnosticsCompleteness::Complete
            && examples_len != self.observed_total.min(self.examples_limit)
        {
            return Err("report complete con esempi incompleti".to_owned());
        }
        let expected_truncated =
            self.observed_total > examples_len && examples_len == self.examples_limit;
        if self.examples_truncated != expected_truncated {
            return Err("examples_truncated incoerente".to_owned());
        }
        let mut indices = std::collections::BTreeSet::new();
        let mut example_causes = BTreeMap::<&str, u64>::new();
        for example in &self.examples {
            if !indices.insert(example.source_index) {
                return Err("source_index duplicato negli esempi".to_owned());
            }
            if !valid_token(&example.cause) || !self.counts.contains_key(&example.cause) {
                return Err("causa esempio assente dai conteggi".to_owned());
            }
            if example
                .column
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.chars().count() > 256)
            {
                return Err("column esempio non valida".to_owned());
            }
            validate_key(example.key.as_ref())?;
            *example_causes.entry(&example.cause).or_default() += 1;
        }
        for (cause, count) in example_causes {
            if count > self.counts[cause] {
                return Err("esempi eccedono il conteggio della causa".to_owned());
            }
        }
        match self.scope {
            RowDiagnosticScope::Read => {
                if self.input_total.is_some()
                    || self.diagnostic_state_counts.is_some()
                    || self.write_outcome.is_some()
                    || self
                        .examples
                        .iter()
                        .any(|example| example.write_state.is_some())
                {
                    return Err("report read contiene campi riservati alla write".to_owned());
                }
            }
            RowDiagnosticScope::Write => self.validate_write(examples_len)?,
        }
        Ok(())
    }

    fn validate_write(&self, _examples_len: u64) -> std::result::Result<(), String> {
        let input_total = self
            .input_total
            .filter(|value| *value > 0)
            .ok_or_else(|| "report write richiede input_total positivo".to_owned())?;
        if self.observed_total > input_total
            || self.examples.iter().any(|e| e.write_state.is_none())
        {
            return Err("report write privo di input totale o stato esempio valido".to_owned());
        }
        let states = self
            .diagnostic_state_counts
            .as_ref()
            .ok_or_else(|| "report write richiede diagnostic_state_counts".to_owned())?;
        let state_sum = states
            .certainly_rejected
            .checked_add(states.certainly_not_attempted)
            .and_then(|sum| sum.checked_add(states.certainly_rolled_back))
            .and_then(|sum| sum.checked_add(states.effect_unknown))
            .ok_or_else(|| "overflow nei diagnostic_state_counts".to_owned())?;
        if state_sum != self.observed_total {
            return Err("diagnostic_state_counts non somma a observed_total".to_owned());
        }
        let mut example_states = WriteDiagnosticStateCounts {
            certainly_rejected: 0,
            certainly_not_attempted: 0,
            certainly_rolled_back: 0,
            effect_unknown: 0,
        };
        for example in &self.examples {
            let count = match example.write_state {
                Some(RowDiagnosticWriteState::CertainlyRejected) => {
                    &mut example_states.certainly_rejected
                }
                Some(RowDiagnosticWriteState::CertainlyNotAttempted) => {
                    &mut example_states.certainly_not_attempted
                }
                Some(RowDiagnosticWriteState::CertainlyRolledBack) => {
                    &mut example_states.certainly_rolled_back
                }
                Some(RowDiagnosticWriteState::EffectUnknown) => &mut example_states.effect_unknown,
                None => return Err("report write privo di stato esempio".to_owned()),
            };
            *count = count
                .checked_add(1)
                .ok_or_else(|| "overflow negli stati degli esempi write".to_owned())?;
        }
        if example_states.certainly_rejected > states.certainly_rejected
            || example_states.certainly_not_attempted > states.certainly_not_attempted
            || example_states.certainly_rolled_back > states.certainly_rolled_back
            || example_states.effect_unknown > states.effect_unknown
        {
            return Err("esempi eccedono diagnostic_state_counts".to_owned());
        }
        let outcome = self
            .write_outcome
            .as_ref()
            .ok_or_else(|| "report write richiede write_outcome".to_owned())?;
        let buckets = [
            (&outcome.certainly_rejected, states.certainly_rejected),
            (
                &outcome.certainly_not_attempted,
                states.certainly_not_attempted,
            ),
            (&outcome.certainly_rolled_back, states.certainly_rolled_back),
            (&outcome.effect_unknown, states.effect_unknown),
        ];
        let mut known_sum = 0_u64;
        let mut diagnosed_unknown = 0_u64;
        let mut all_known = true;
        for (bucket, diagnosed) in buckets {
            match bucket {
                KnownOrUnknownCount::Known { value } => {
                    if diagnosed > *value {
                        return Err("diagnostica eccede un bucket write noto".to_owned());
                    }
                    known_sum = known_sum
                        .checked_add(*value)
                        .ok_or_else(|| "overflow nei bucket write".to_owned())?;
                }
                KnownOrUnknownCount::Unknown => {
                    all_known = false;
                    diagnosed_unknown = diagnosed_unknown
                        .checked_add(diagnosed)
                        .ok_or_else(|| "overflow nei bucket write unknown".to_owned())?;
                }
            }
        }
        if known_sum > input_total
            || known_sum
                .checked_add(diagnosed_unknown)
                .is_none_or(|sum| sum > input_total)
            || (all_known && known_sum != input_total)
        {
            return Err("partizione write_outcome incoerente con input_total".to_owned());
        }
        Ok(())
    }
}

fn valid_token(value: &str) -> bool {
    value.len() <= 128
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.as_bytes().iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
        && !value.as_bytes().windows(2).any(|pair| {
            matches!(pair[0], b'.' | b'_' | b'-') && matches!(pair[1], b'.' | b'_' | b'-')
        })
        && value
            .as_bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn validate_knowledge_limits(values: Option<&[String]>) -> std::result::Result<(), String> {
    let values = values
        .filter(|values| !values.is_empty())
        .ok_or_else(|| "report non complete richiede knowledge_limits".to_owned())?;
    let mut unique = std::collections::BTreeSet::new();
    if values
        .iter()
        .any(|value| !valid_token(value) || !unique.insert(value))
    {
        return Err("knowledge_limits non validi o duplicati".to_owned());
    }
    Ok(())
}

fn validate_key(key: Option<&RowDiagnosticKey>) -> std::result::Result<(), String> {
    let Some(key) = key else {
        return Ok(());
    };
    if key.field.is_empty() || key.field.chars().count() > 256 {
        return Err("campo chiave diagnostica non valido".to_owned());
    }
    match (&key.state, &key.value) {
        (RowDiagnosticKeyState::Value, Some(RowDiagnosticKeyValue::String(value)))
            if value.chars().count() <= 1024 =>
        {
            Ok(())
        }
        (RowDiagnosticKeyState::Value, Some(RowDiagnosticKeyValue::Integer(value)))
            if (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(value) =>
        {
            Ok(())
        }
        // I due rami restano distinti perche' codificano regole diverse del
        // contratto (valore booleano ammesso vs assenza di valore per stato
        // redatto/indisponibile); fonderli ne perderebbe la tracciabilita'.
        #[allow(clippy::match_same_arms)]
        (RowDiagnosticKeyState::Value, Some(RowDiagnosticKeyValue::Boolean(_))) => Ok(()),
        (RowDiagnosticKeyState::Redacted | RowDiagnosticKeyState::Unavailable, None) => Ok(()),
        _ => Err("stato/value della chiave diagnostica incoerente".to_owned()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeserializableRowDiagnostics {
    contract: String,
    scope: RowDiagnosticScope,
    index_basis: String,
    completeness: RowDiagnosticsCompleteness,
    #[serde(default)]
    knowledge_limits: Option<Vec<String>>,
    observed_total: u64,
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    input_total: Option<u64>,
    counts: BTreeMap<String, u64>,
    examples_limit: u64,
    examples_truncated: bool,
    examples: Vec<RowDiagnosticExample>,
    #[serde(default)]
    diagnostic_state_counts: Option<WriteDiagnosticStateCounts>,
    #[serde(default)]
    write_outcome: Option<RowDiagnosticWriteOutcome>,
}

impl<'de> Deserialize<'de> for RowDiagnostics {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = DeserializableRowDiagnostics::deserialize(deserializer)?;
        let diagnostics = Self {
            contract: raw.contract,
            scope: raw.scope,
            index_basis: raw.index_basis,
            completeness: raw.completeness,
            knowledge_limits: raw.knowledge_limits,
            observed_total: raw.observed_total,
            total: raw.total,
            input_total: raw.input_total,
            counts: raw.counts,
            examples_limit: raw.examples_limit,
            examples_truncated: raw.examples_truncated,
            examples: raw.examples,
            diagnostic_state_counts: raw.diagnostic_state_counts,
            write_outcome: raw.write_outcome,
        };
        diagnostics.validate().map_err(serde::de::Error::custom)?;
        Ok(diagnostics)
    }
}

#[derive(Serialize)]
struct SerializableRowDiagnostics<'a> {
    contract: &'a str,
    scope: RowDiagnosticScope,
    index_basis: &'a str,
    completeness: RowDiagnosticsCompleteness,
    #[serde(skip_serializing_if = "Option::is_none")]
    knowledge_limits: &'a Option<Vec<String>>,
    observed_total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: &'a Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_total: &'a Option<u64>,
    counts: &'a BTreeMap<String, u64>,
    examples_limit: u64,
    examples_truncated: bool,
    examples: &'a [RowDiagnosticExample],
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic_state_counts: &'a Option<WriteDiagnosticStateCounts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    write_outcome: &'a Option<RowDiagnosticWriteOutcome>,
}

impl Serialize for RowDiagnostics {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        SerializableRowDiagnostics {
            contract: &self.contract,
            scope: self.scope,
            index_basis: &self.index_basis,
            completeness: self.completeness,
            knowledge_limits: &self.knowledge_limits,
            observed_total: self.observed_total,
            total: &self.total,
            input_total: &self.input_total,
            counts: &self.counts,
            examples_limit: self.examples_limit,
            examples_truncated: self.examples_truncated,
            examples: &self.examples,
            diagnostic_state_counts: &self.diagnostic_state_counts,
            write_outcome: &self.write_outcome,
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_read() -> RowDiagnostics {
        RowDiagnostics {
            contract: ROW_DIAGNOSTICS_CONTRACT.to_owned(),
            scope: RowDiagnosticScope::Read,
            index_basis: ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
            completeness: RowDiagnosticsCompleteness::Complete,
            knowledge_limits: None,
            observed_total: 1,
            total: Some(1),
            input_total: None,
            counts: BTreeMap::from([("conversion.invalid_date".to_owned(), 1)]),
            examples_limit: 10,
            examples_truncated: false,
            examples: vec![RowDiagnosticExample {
                source_index: 4,
                cause: "conversion.invalid_date".to_owned(),
                column: Some("effective_date".to_owned()),
                key: None,
                write_state: None,
            }],
            diagnostic_state_counts: None,
            write_outcome: None,
        }
    }

    #[test]
    fn invalid_documents_are_not_serializable() {
        let mut report = valid_read();
        report.observed_total = 2;
        assert!(serde_json::to_value(&report).is_err());

        let mut report = valid_read();
        report.examples[0].write_state = Some(RowDiagnosticWriteState::CertainlyRejected);
        assert!(serde_json::to_value(&report).is_err());

        let mut report = valid_read();
        report.examples[0].source_index = 7;
        report.examples.push(report.examples[0].clone());
        report.observed_total = 2;
        report.total = Some(2);
        report
            .counts
            .insert("conversion.invalid_date".to_owned(), 2);
        assert!(serde_json::to_value(&report).is_err());
    }

    #[test]
    fn invalid_documents_are_not_deserializable() {
        let mut document = serde_json::to_value(valid_read()).unwrap();
        document["contract"] = serde_json::Value::String("wrong".to_owned());
        assert!(serde_json::from_value::<RowDiagnostics>(document).is_err());
    }

    #[test]
    fn unicode_limits_count_characters() {
        let mut report = valid_read();
        report.examples[0].column = Some("é".repeat(256));
        report.examples[0].key = Some(RowDiagnosticKey {
            field: "名".repeat(256),
            state: RowDiagnosticKeyState::Value,
            value: Some(RowDiagnosticKeyValue::String("😀".repeat(1024))),
        });
        assert!(report.validate().is_ok());
    }

    #[test]
    fn diagnostic_column_attestation_accepts_unicode_and_hides_invalid_names() {
        assert_eq!(
            RowDiagnosticColumn::attest("citta_ðŸŒ")
                .into_option()
                .as_deref(),
            Some("citta_ðŸŒ")
        );
        assert_eq!(RowDiagnosticColumn::attest("").into_option(), None);
        assert_eq!(
            RowDiagnosticColumn::attest("x".repeat(257)).into_option(),
            None
        );
    }

    #[test]
    fn write_examples_cannot_exceed_their_diagnostic_state() {
        let mut report = valid_read();
        report.scope = RowDiagnosticScope::Write;
        report.input_total = Some(1);
        report.examples[0].write_state = Some(RowDiagnosticWriteState::CertainlyRejected);
        report.diagnostic_state_counts = Some(WriteDiagnosticStateCounts {
            certainly_rejected: 0,
            certainly_not_attempted: 1,
            certainly_rolled_back: 0,
            effect_unknown: 0,
        });
        report.write_outcome = Some(RowDiagnosticWriteOutcome {
            certainly_rejected: KnownOrUnknownCount::Known { value: 0 },
            certainly_not_attempted: KnownOrUnknownCount::Known { value: 1 },
            certainly_rolled_back: KnownOrUnknownCount::Known { value: 0 },
            effect_unknown: KnownOrUnknownCount::Known { value: 0 },
        });
        assert!(report.validate().is_err());
    }
}
