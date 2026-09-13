//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

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
