use super::*;

#[test]
fn typed_domains_round_trip_without_precision_loss() {
    let domains = BTreeMap::from([
        (
            "stato_è".into(),
            Domain::Coded {
                code_type: CodeType::Int64,
                values: vec![CodedValue {
                    code: Code::Int64(i64::MAX),
                    description: Some("Città 水".into()),
                }],
            },
        ),
        (
            "range".into(),
            Domain::Range {
                min: Code::Float64(-12.5),
                max: Code::Float64(105.25),
                min_inclusive: true,
                max_inclusive: false,
            },
        ),
        (
            "vuoto".into(),
            Domain::Coded {
                code_type: CodeType::Int32,
                values: vec![],
            },
        ),
    ]);
    assert_eq!(
        decode_domains(&encode_domains(&domains).unwrap()).unwrap(),
        domains
    );
    assert!(decode_domains("{}").unwrap().is_empty());
}

#[test]
fn malformed_values_are_typed_errors_without_payload() {
    for input in [
        r#"{"secret":{"kind":"range","min":{"type":"int32","value":9},"max":{"type":"int32","value":1},"min_inclusive":true,"max_inclusive":true}}"#,
        r#"{"secret":{"kind":"coded","code_type":"int32","values":[{"code":{"type":"utf8","value":"private"},"description":null}]}}"#,
        r#"{"secret":{"kind":"unknown"}}"#,
    ] {
        let error = decode_domains(input).unwrap_err();
        assert!(!error.to_string().contains("secret"));
        assert!(!error.to_string().contains("private"));
    }
}

#[test]
fn subtype_defaults_and_duplicates_are_validated() {
    let value = Subtypes {
        field: "tipo".into(),
        default: Some(1),
        values: vec![Subtype {
            code: 1,
            name: "Città".into(),
            domains: BTreeMap::from([("misura".into(), "range".into())]),
        }],
    };
    assert_eq!(
        decode_subtypes(&encode_subtypes(&value).unwrap()).unwrap(),
        value
    );
    let mut duplicate = value.clone();
    duplicate.values.extend(value.values);
    assert!(encode_subtypes(&duplicate).is_err());
    duplicate.values.clear();
    assert!(encode_subtypes(&duplicate).is_err());
}
