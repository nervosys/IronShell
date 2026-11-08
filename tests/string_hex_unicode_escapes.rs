//! `\xHH` and `\u{H..H}` escapes in double-quoted strings, run through the real
//! binary. Before this, both were kept verbatim (`"\x41"` printed `\x41`). A
//! malformed escape keeps its backslash, as any unknown escape does.
//!
//! `\uHHHH` (without braces) is not an escape the lexer knows, and it is not
//! added here: `"A"` still prints `A`.
//! A script echoes its last value, so each one ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_escape_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("escape.ish");
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
fn a_two_digit_hex_escape_names_its_code_point() {
    let (out, err, code) = run_script("hex", "\"a\\x41b\\x7e\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "aAb~", "stdout was {out:?}");
}

#[test]
fn a_braced_unicode_escape_takes_one_to_six_hex_digits() {
    let (out, err, code) = run_script("uni", "\"\\u{41}\\u{1F600}\\u{0000000041}x\"\n");
    assert_eq!(code, 0, "{err}");
    // The six-digit form is over six digits here (ten), so it is malformed and
    // keeps its backslash; only the first two escapes decode.
    assert_eq!(out, "A\u{1F600}\\u{0000000041}x", "stdout was {out:?}");
}

#[test]
fn an_escape_that_names_no_scalar_keeps_its_backslash() {
    // 0x110000 is past U+10FFFF, and a surrogate is not a scalar value.
    let (out, err, code) = run_script("bad", "\"\\u{110000}|\\u{D800}|\\xZZ|\\x4|\\u{}\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out, "\\u{110000}|\\u{D800}|\\xZZ|\\x4|\\u{}",
        "stdout was {out:?}"
    );
}

#[test]
fn an_escape_decodes_inside_an_interpolated_string() {
    let (out, err, code) = run_script("interp", "n = 2\n\"\\x41${n}\\u{42}\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "A2B", "stdout was {out:?}");
}
