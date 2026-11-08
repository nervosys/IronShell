//! `let x: T = value` checks the value against the annotation, run through the
//! real binary. Before this, the annotation was parsed and discarded:
//! `let x: Int = "s"` then `x + 1` printed `s1`, and `let x: String = 42` bound
//! an Int. A script echoes its last value, so each one ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_letty_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("letty.ish");
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
fn a_string_bound_to_an_int_annotation_is_refused() {
    let (out, err, code) = run_script("str_int", "let x: Int = \"s\"\nx + 1\n");
    assert_ne!(code, 0, "ran with stdout {out:?}");
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(
        err.contains("let x: expected Int, got String"),
        "no declared/actual message: {err}"
    );
    assert!(!out.contains("s1"), "the bad value still ran: {out:?}");
}

#[test]
fn an_int_bound_to_a_string_annotation_is_refused() {
    let (_, err, code) = run_script("int_str", "let x: String = 42\nx\n");
    assert_ne!(code, 0);
    assert!(
        err.contains("let x: expected String, got Int"),
        "no declared/actual message: {err}"
    );
}

#[test]
fn an_int_is_refused_for_a_float_annotation_with_a_decimal_hint() {
    let (_, err, code) = run_script("int_float", "let f: Float = 1\nf\n");
    assert_ne!(code, 0);
    assert!(
        err.contains("let f: expected Float, got Int"),
        "no message: {err}"
    );
    assert!(err.contains("decimal point"), "no decimal hint: {err}");
}

#[test]
fn a_matching_annotation_binds_as_before() {
    let (out, err, code) = run_script(
        "match",
        "let x: Int = 5\nlet mut y: Float = 1.5\nlet s: String = \"a\"\nlet b: Bool = true\nlet a: Array = [1]\nlet r: Record = {k: 1}\nlet n: Null = null\nx + 1\n",
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "6");
}

#[test]
fn an_unknown_annotation_is_not_checked() {
    let (out, err, code) = run_script("unknown", "let z: Widget = 1\nz\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "1");
}

#[test]
fn an_unannotated_binding_accepts_any_value() {
    let (out, err, code) = run_script("plain", "let x = \"s\"\nx\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "s");
}
