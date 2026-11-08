//! The hints for a lambda parameter with a default value or a type annotation
//! say what is wrong, run through the real binary. Before this, `fn(a, b = 10)`
//! and `fn(x: Int)` were both answered with "use `==` to compare; `=` assigns",
//! which is wrong for both: lambda parameters take no defaults and no types.
//! The `==` hint still appears where `=` sits in a comparison.
//! A script echoes its last value, so each one ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_lamhint_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("hint.ish");
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
fn a_default_parameter_value_is_not_called_a_comparison() {
    let (_, err, code) = run_script("default", "g = fn(a, b = 10) => a + b\ng(1)\n");
    assert_ne!(code, 0);
    assert!(err.contains("E_PARSE"), "{err}");
    assert!(
        err.contains("lambda parameters take no default values"),
        "no default-value hint: {err}"
    );
    assert!(
        err.contains("check for null in the body"),
        "no null-check advice: {err}"
    );
    assert!(!err.contains("use `==`"), "still says `==`: {err}");
}

#[test]
fn a_parameter_type_annotation_is_not_called_a_comparison() {
    let (_, err, code) = run_script("typed", "h = fn(x: Int) => x\nh(1)\n");
    assert_ne!(code, 0);
    assert!(err.contains("E_PARSE"), "{err}");
    assert!(
        err.contains("lambda parameters take no type annotations"),
        "no annotation hint: {err}"
    );
    assert!(
        err.contains("let x: Type = value"),
        "no pointer to let: {err}"
    );
    assert!(!err.contains("use `==`"), "still says `==`: {err}");
}

#[test]
fn an_equals_sign_in_a_comparison_still_gets_the_comparison_hint() {
    let (_, err, code) = run_script("cmp", "[1, 2] | filter(fn(r) => r = 1)\n");
    assert_ne!(code, 0);
    assert!(
        err.contains("use `==` to compare; `=` assigns"),
        "no comparison hint: {err}"
    );
}

#[test]
fn a_parameter_list_that_is_valid_still_runs() {
    let (out, err, code) = run_script("valid", "f = fn(a, b) => a + b\nf(1, 2)\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "3");
}
