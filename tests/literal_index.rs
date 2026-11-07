//! Indexing a string or record literal written directly before the `[`,
//! run through the real binary.
//!
//! Reproduced on 14.0.0 in a script file: `"hello"[0]` printed `[0]` and
//! `{k: 1}["k"]` printed `[k]`, both with exit 0. The parenthesised forms
//! `("hello")[0]` and `({k: 1})["k"]` already indexed. A script echoes its
//! last value, so each one ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_litidx_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("literal_index.ish");
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

#[test]
fn a_string_literal_indexed_directly_returns_the_character() {
    assert_prints("str0", "\"hello\"[0]\n", "h");
}

#[test]
fn a_string_literal_indexed_directly_at_a_later_position() {
    assert_prints("str1", "\"hello\"[1]\n", "e");
}

#[test]
fn a_single_quoted_string_literal_indexes_directly() {
    assert_prints("strq", "'hello'[4]\n", "o");
}

#[test]
fn a_record_literal_indexed_directly_returns_the_value() {
    assert_prints("rec", "{k: 1}[\"k\"]\n", "1");
}

#[test]
fn a_record_literal_indexed_directly_then_indexed_again() {
    assert_prints("recarr", "{a: [5, 6]}[\"a\"][1]\n", "6");
}

#[test]
fn a_parenthesised_string_literal_still_indexes() {
    assert_prints("parenstr", "(\"hello\")[0]\n", "h");
}

#[test]
fn a_parenthesised_record_literal_still_indexes() {
    assert_prints("parenrec", "({k: 1})[\"k\"]\n", "1");
}

#[test]
fn a_block_closing_brace_followed_by_an_array_is_not_an_index() {
    // The `}` ends an `if` block, not a record literal, so `[1, 2]` is the
    // array that follows it and the statement's value is that array.
    assert_prints("block", "if true { 5 }[1, 2]\n", "[1, 2]");
}
