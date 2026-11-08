//! The variable a `catch` binds exposes an error's fields at the top level, as
//! well as under `error`, and `is_error` recognises it, run through the real
//! binary. Before this, `catch e { e.code }` failed with "field 'code' not found
//! in record", `is_error(e)` was false inside a catch, and a zero divisor's
//! `builtin` field was empty. A script echoes its last value, so each one ends
//! in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_caught_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file in a fresh temp directory and returns
/// (stdout, stderr, exit code). The directory holds no `nope.txt`.
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("caught.ish");
    std::fs::write(&script, source).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg(&script)
        .current_dir(&dir)
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
fn a_caught_error_exposes_its_code_at_the_top_level() {
    let (out, err, code) = run_script(
        "top",
        "r = try { file.read(\"nope.txt\") } catch e { e.code }\nr\n",
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "E_NOT_FOUND", "stdout was {out:?}");
}

#[test]
fn the_nested_error_field_still_works() {
    let (out, err, code) = run_script(
        "nested",
        "r = try { file.read(\"nope.txt\") } catch e { e.error.code }\nr\n",
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "E_NOT_FOUND", "stdout was {out:?}");
}

#[test]
fn a_caught_error_carries_message_hint_and_retryable_at_the_top_level() {
    let (out, err, code) = run_script(
        "fields",
        "try { 10 / 0 } catch e { [e.message, e.hint, e.retryable] }\n",
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("division by zero"), "message missing: {out}");
    assert!(out.contains("divisor"), "hint missing: {out}");
    assert!(out.contains("false"), "retryable missing: {out}");
}

#[test]
fn is_error_is_true_for_a_caught_error() {
    let (out, err, code) = run_script(
        "is_err",
        "try { file.read(\"nope.txt\") } catch e { is_error(e) }\n",
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "true", "stdout was {out:?}");
}

#[test]
fn is_error_is_still_false_for_a_plain_value() {
    let (out, err, code) = run_script("plain", "is_error(42)\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "false", "stdout was {out:?}");
}

#[test]
fn a_zero_divisor_names_its_operator_in_builtin() {
    let (out, err, code) = run_script("div", "try { 10 / 0 } catch e { e.builtin }\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "operator Div", "stdout was {out:?}");
}
