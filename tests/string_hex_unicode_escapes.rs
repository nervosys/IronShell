//! `\xHH` and `\u{H..H}` escapes in double-quoted strings, run through the real
//! binary. Before this, both were kept verbatim (`"\x41"` printed `\x41`). A
//! malformed `\x` keeps its backslash, as any unknown escape does; a malformed
//! `\u` is E_PARSE.
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
    let (out, err, code) = run_script("uni", "\"\\u{41}\\u{1F600}x\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "A\u{1F600}x", "stdout was {out:?}");
}

#[test]
fn a_malformed_unicode_escape_is_refused_with_its_position() {
    // The six-digit form is over six digits here (ten): malformed, so refused
    // as E_PARSE naming the escape, rather than printed with its backslash.
    let (_out, err, code) = run_script("long", "\"\\u{0000000041}x\"\n");
    assert_ne!(code, 0, "{err}");
    assert!(err.contains("E_PARSE"), "{err}");
    assert!(
        err.contains("invalid unicode escape `\\u{0000000041}` at line 1, column 2"),
        "{err}"
    );
}

#[test]
fn an_unknown_hex_escape_still_keeps_its_backslash() {
    // \x is not refused; only a malformed \u is. Neither decodes here.
    let (out, err, code) = run_script("bad", "\"\\xZZ|\\x4\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "\\xZZ|\\x4", "stdout was {out:?}");
}

#[test]
fn an_escape_decodes_inside_an_interpolated_string() {
    let (out, err, code) = run_script("interp", "n = 2\n\"\\x41${n}\\u{42}\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "A2B", "stdout was {out:?}");
}
