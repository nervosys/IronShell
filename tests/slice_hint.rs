//! `a[start:end]` is refused with a hint that names the slice builtin, run
//! through the real binary.
//!
//! Reproduced on 14.0.0: `a[0:2]` failed with E_PARSE "expected ']' after
//! index" and the generic hint, which says nothing about slicing. The hint now
//! says there is no slice syntax and names `slice`, which the signature table
//! documents as `subject | slice(start, end)`. Other parse errors keep the
//! generic hint.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_slicehint_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("slice_hint.ish");
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
fn a_colon_index_is_still_a_parse_error() {
    let (_, err, code) = run_script("still", "a = [1, 2, 3, 4]\na[0:2]\n");
    assert_eq!(code, 2, "stderr: {err}");
    assert!(err.contains("E_PARSE"), "stderr: {err}");
    assert!(err.contains("expected ']' after index"), "stderr: {err}");
}

#[test]
fn a_colon_index_hint_says_there_is_no_slice_syntax_and_names_slice() {
    let (_, err, code) = run_script("hint", "a = [1, 2, 3, 4]\na[0:2]\n");
    assert_eq!(code, 2, "stderr: {err}");
    assert!(
        err.contains("hint: IronShell has no slice syntax"),
        "the hint must say there is no slice syntax: {err}"
    );
    assert!(
        err.contains("slice(start, end)"),
        "the hint must name the slice builtin and its arguments: {err}"
    );
    assert!(
        !err.contains("fix the syntax at the reported line"),
        "the generic hint must not be the one shown: {err}"
    );
}

#[test]
fn the_named_builtin_does_the_slice() {
    let (out, err, code) = run_script("works", "[1, 2, 3, 4] | slice(0, 2)\n");
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[1, 2]");
}

#[test]
fn an_ordinary_parse_error_keeps_the_generic_hint() {
    let (_, err, code) = run_script("generic", "x = (1\n");
    assert_eq!(code, 2, "stderr: {err}");
    assert!(
        err.contains("fix the syntax at the reported line"),
        "other parse errors keep the generic hint: {err}"
    );
    assert!(!err.contains("no slice syntax"), "stderr: {err}");
}
