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
