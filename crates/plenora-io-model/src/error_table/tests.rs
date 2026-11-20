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
