//! Redaction hides secret values, not the type names in a signature.
//!
//! The agent-facing output of `approve` showed `approve(token: [REDACTED] ->
//! Record}`: the assignment shape read `token: String)` as a token whose value
//! was `String)`, and replaced it, closing paren and all. A signature type name
//! is a description, so it is left. A real secret after the same key is still
//! redacted, and the paren that follows a secret is kept.

use iron_shell::safety::{redact_str, REDACTION_MARKER};
use iron_shell::signature::{signature_of, Ty};

#[test]
fn the_approve_signature_is_not_redacted() {
    let sig = signature_of("approve").expect("approve is declared");
    let rendered = sig.render();
    assert_eq!(rendered, "approve(token: String) -> Record");
    let (out, changed) = redact_str(&rendered);
    assert_eq!(out, rendered);
    assert!(!changed, "nothing in a signature is a secret");
}

#[test]
fn a_secret_after_the_same_key_is_still_redacted() {
    let (out, changed) = redact_str("token: hunter22");
    assert!(changed);
    assert!(!out.contains("hunter22"), "{out}");
    assert!(out.contains(REDACTION_MARKER), "{out}");
}

#[test]
fn the_paren_after_a_redacted_secret_is_kept() {
    let (out, _) = redact_str("(password: hunter2)");
    assert!(!out.contains("hunter2"), "{out}");
    assert_eq!(out, "(password: [REDACTED])");
}

#[test]
fn a_type_word_is_only_exempt_as_a_whole_word() {
    assert!(Ty::is_type_name("String"));
    assert!(Ty::is_type_name("Number"));
    assert!(!Ty::is_type_name("hunter2"));
    assert!(!Ty::is_type_name("Strings"));
    // `secret: Stringy` is a value, not the type `String`.
    let (out, changed) = redact_str("secret: Stringy1");
    assert!(changed && !out.contains("Stringy1"), "{out}");
}
