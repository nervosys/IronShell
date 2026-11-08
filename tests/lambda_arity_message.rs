//! A lambda called with the wrong number of arguments names itself and its
//! parameters, and its hint is about lambdas, not builtins. Before this, `f(1)`
//! for `f = fn(a, b) => a + b` said "lambda arity mismatch or missing input" with
//! "check this builtin's required argument count and types".
//! A script echoes its last value, so each one ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_arity_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("arity.ish");
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
fn a_named_lambda_called_with_too_few_arguments_names_its_variable() {
    let (_, err, code) = run_script("named", "f = fn(a, b) => a + b\nf(1)\n");
    assert_ne!(code, 0);
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(
        err.contains("f takes 2 arguments (a, b), got 1"),
        "no arity message: {err}"
    );
    assert!(
        !err.contains("lambda arity mismatch"),
        "old message kept: {err}"
    );
    assert!(
        !err.contains("builtin"),
        "hint still talks of builtins: {err}"
    );
}

#[test]
fn an_unnamed_lambda_is_called_lambda() {
    let (_, err, code) = run_script("unnamed", "(fn(a, b) => a + b)(1)\n");
    assert_ne!(code, 0);
    assert!(
        err.contains("lambda takes 2 arguments (a, b), got 1"),
        "no arity message: {err}"
    );
}

#[test]
fn a_call_with_the_right_count_still_runs() {
    let (out, err, code) = run_script("ok", "f = fn(a, b) => a + b\nf(1, 2)\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "3");
}
