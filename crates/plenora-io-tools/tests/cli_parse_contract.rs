//! Malformed options fail before any dataset can be opened.
use plenora_io_tools::parse;

#[test]
fn value_options_require_values_and_numeric_selectors_require_integers() {
    for flag in [
        "--assume-crs",
        "--format",
        "--output",
        "--layer",
        "--limit",
        "--from",
        "--to",
        "--opt",
        "--in-opt",
        "--out-opt",
        "--deadline-ms",
    ] {
        let (code, envelope) = parse(&[flag.to_owned()])
            .err()
            .expect("invalid options must fail");
        assert_eq!(code, 2, "{flag}");
        assert_eq!(envelope["status"], "error", "{flag}");
    }
    for flag in ["--layer", "--limit", "--deadline-ms"] {
        for value in ["secret-invalid-number", "-1", "18446744073709551616"] {
            let (code, envelope) = parse(&[flag.to_owned(), value.to_owned()])
                .err()
                .expect("invalid options must fail");
            assert_eq!(code, 2);
            assert!(!envelope.to_string().contains(value));
        }
    }
}

#[test]
fn format_options_allow_distinct_keys_but_reject_ambiguous_duplicates() {
    for flag in ["--opt", "--in-opt", "--out-opt"] {
        let arguments = [flag, "first=one", flag, "second=two"].map(str::to_owned);
        assert!(parse(&arguments).is_ok());
        let duplicate = [flag, "first=one", flag, "first=two"].map(str::to_owned);
        assert_eq!(
            parse(&duplicate)
                .err()
                .expect("invalid options must fail")
                .0,
            2
        );
    }
    assert!(parse(&["--layer", "0", "--limit", "1"].map(str::to_owned)).is_ok());

    assert_eq!(
        parse(&["--limit", "0"].map(str::to_owned))
            .err()
            .expect("invalid options must fail")
            .0,
        2
    );
}
