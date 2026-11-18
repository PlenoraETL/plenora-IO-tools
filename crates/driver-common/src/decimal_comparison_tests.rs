#[test]
fn compares_values_in_both_directions() {
    for text in ["1", "1.50", "1.5e0", "0.125", "-0", "1000.00", "1e3"] {
        assert!(!super::decimal_value_changes(text), "{text}");
    }
    for text in ["9007199254740993", "1.00000000000000001", "1e-400"] {
        assert!(super::decimal_value_changes(text), "{text}");
    }
}
