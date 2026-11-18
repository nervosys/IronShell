//! Round 54c: messages and small correctness.

use iron_shell::safety::{redact_str, take_redaction_count};

#[test]
fn a_key_value_secret_whose_value_is_a_bare_key_counts_once() {
    let _ = take_redaction_count();
    let (out, changed) =
        redact_str("key sk-test-123456789abcdef and API_KEY=sk-test-123456789abcdef");
    assert!(changed);
    assert_eq!(out.matches("[REDACTED]").count(), 2);
    assert_eq!(take_redaction_count(), 2);
}

fn refusal(src: &str) -> String {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    match iron_shell::eval::eval_program(&stmts, &mut env) {
        Ok(_) => panic!("{src}: expected a refusal"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn an_iso_week_without_a_weekday_says_so() {
    let text = refusal(r#"date_format("2024-W09", "%Y-%m-%d")"#);
    assert!(text.contains("ISO week without a weekday"), "{text}");
    assert!(text.contains("2024-W09-1"), "{text}");
    assert!(!text.contains("zero-padded"), "{text}");
    let text = refusal(r#"date_format("2024W093", "%Y-%m-%d")"#);
    assert!(text.contains("basic ISO week form"), "{text}");
}
