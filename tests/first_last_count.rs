//! `first(n)` and `last(n)` return an Array for an explicit count, including
//! n=1, run through the real binary.
//!
//! Reproduced on 14.0.0: `[10,20,30] | first(1)` printed the scalar 10, so
//! `[10,20,30] | first(1) | map(fn(x) => x * 2)` failed with E_BAD_ARG "map:
//! expected Array as the piped subject ... got Int". The ontology already said
//! `first(n)` returns the first n as an array. The bare call still returns the
//! element. A script echoes its last value, so each one ends in a bare
//! expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_firstlast_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("firstlast.ish");
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
fn first_with_count_one_is_an_array() {
    assert_prints("first1", "[10,20,30] | first(1)\n", "[10]");
}

#[test]
fn last_with_count_one_is_an_array() {
    assert_prints("last1", "[10,20,30] | last(1)\n", "[30]");
}

#[test]
fn first_with_count_one_can_feed_map() {
    assert_prints(
        "first_map",
        "[10,20,30] | first(1) | map(fn(x) => x * 2)\n",
        "[20]",
    );
}

#[test]
fn last_with_count_one_can_feed_map() {
    assert_prints(
        "last_map",
        "[10,20,30] | last(1) | map(fn(x) => x * 2)\n",
        "[60]",
    );
}

#[test]
fn bare_first_still_returns_the_element() {
    assert_prints("bare_first", "[10,20,30] | first\n", "10");
}

#[test]
fn bare_last_still_returns_the_element() {
    assert_prints("bare_last", "[10,20,30] | last\n", "30");
}

#[test]
fn counts_other_than_one_are_unchanged() {
    assert_prints("two", "[10,20,30] | first(2)\n", "[10, 20]");
    assert_prints("two_last", "[10,20,30] | last(2)\n", "[20, 30]");
}

#[test]
fn an_empty_array_with_count_one_is_empty() {
    assert_prints("empty", "[] | first(1)\n", "[]");
}
