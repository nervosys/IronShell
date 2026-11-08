//! A string literal with no closing quote is E_PARSE, run through the real
//! binary.
//!
//! Reproduced on 14.0.0: a script holding `"unterminated` printed
//! `unterminated` with exit 0, and `'unterminated` did the same. The lexer ran
//! to the end of the input and accepted whatever it had read. The error now
//! points at the opening quote, because that is where the literal starts. A
//! string closed by its own quote, including one that holds the other quote
//! character, is unchanged. A script echoes its last value, so each valid one
//! ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_unterm_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("string.ish");
    std::fs::write(&script, source).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg(&script)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn assert_prints(tag: &str, source: &str, expected: &str) {
    let (out, err, code) = run_script(tag, source);
    assert_eq!(code, 0, "source {source:?} failed: {err}");
    assert_eq!(out, expected, "source {source:?}");
}

fn assert_unterminated(tag: &str, source: &str, at: &str) {
    let (out, err, code) = run_script(tag, source);
    assert_ne!(code, 0, "source {source:?} ran; stdout {out:?}");
    assert!(err.contains("E_PARSE"), "source {source:?}: {err}");
    assert!(
        err.contains(&format!("unterminated string at line {at}")),
        "source {source:?} does not point at the opening quote: {err}"
    );
    assert!(
        out.is_empty(),
        "source {source:?} printed {out:?} before refusing"
    );
}

#[test]
fn an_unterminated_double_quoted_string_is_refused_at_its_quote() {
    assert_unterminated("dq", "\"unterminated\n", "1, column 1");
}

#[test]
fn an_unterminated_single_quoted_string_is_refused_at_its_quote() {
    assert_unterminated("sq", "'unterminated\n", "1, column 1");
}

#[test]
fn the_position_is_the_opening_quote_not_the_start_of_the_file() {
    assert_unterminated("pos", "x = 1\ny = 2\nz = \"abc\n", "3, column 5");
}

#[test]
fn an_escaped_quote_does_not_close_a_double_quoted_string() {
    assert_unterminated("esc", "\"abc\\\"\n", "1, column 1");
}

#[test]
fn a_closed_double_quoted_string_still_works() {
    assert_prints("dq_ok", "\"abc\"\n", "abc");
}

#[test]
fn a_closed_single_quoted_string_still_works() {
    assert_prints("sq_ok", "'it'\n", "it");
}

#[test]
fn a_single_quote_inside_double_quotes_does_not_open_a_string() {
    assert_prints("mixed", "\"it's\"\n", "it's");
}
