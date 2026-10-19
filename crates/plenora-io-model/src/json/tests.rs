use super::*;

#[test]
fn duplicati_rifiutati_a_ogni_livello() {
    for text in [
        r#"{"segreto":"valore","segreto":2}"#,
        r#"{"a":{"segreto":1,"segreto":2}}"#,
        r#"[{"segreto":1,"segreto":2}]"#,
        r#"{"a":1,"a":2}"#,
    ] {
        let error = ensure_no_duplicate_keys(text.as_bytes()).unwrap_err();
        assert!(!error.contains("segreto"));
        assert!(!error.contains("valore"));
    }
}

#[test]
fn json_valido_e_documento_completo() {
    for text in [r#"{"a":1,"b":[{"a":2},true,null,"testo",1.5]}"#, "[]", "42"] {
        assert!(ensure_no_duplicate_keys(text.as_bytes()).is_ok());
    }
    for text in ["{} {}", "{", r#"{"segreto":invalid}"#] {
        assert!(ensure_no_duplicate_keys(text.as_bytes()).is_err());
    }
}
