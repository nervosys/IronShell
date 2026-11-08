//! A string literal inside a `${ ... }` hole, and a hole inside that literal,
//! run through the real binary. Before the lexer tracked holes, `"x ${"b"} c"`
//! ended its string at the inner quote and failed with "unknown builtin: b", and
//! `"x ${"in ${n * 2} q"} y"` failed with "unknown character '$'".
//! A script echoes its last value, so each one ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_interp_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("interp.ish");
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

#[test]
fn a_string_literal_inside_a_hole_is_one_value() {
    let (out, err, code) = run_script("lit", "\"x ${\"b\"} c\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "x b c", "stdout was {out:?}");
}

#[test]
fn a_hole_nests_inside_a_string_inside_a_hole() {
    let (out, err, code) = run_script("nest", "n = 3\n\"x ${\"in ${n * 2} q\"} y\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "x in 6 q y", "stdout was {out:?}");
}

#[test]
fn a_closing_brace_inside_a_nested_string_does_not_end_the_hole() {
    let (out, err, code) = run_script("brace", "\"a${\"}\"}b\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "a}b", "stdout was {out:?}");
}

#[test]
fn an_escaped_quote_inside_a_hole_stays_in_its_string() {
    let (out, err, code) = run_script("esc", "\"${\"a\\\"b\"}\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "a\"b", "stdout was {out:?}");
}

#[test]
fn a_single_quoted_hole_still_interpolates() {
    let (out, err, code) = run_script("single", "'single ${1+1}'\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "single 2", "stdout was {out:?}");
}
