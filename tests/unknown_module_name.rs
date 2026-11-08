//! `name.member(...)` where `name` is neither bound nor a module is the
//! unknown-name error, run through the real binary.
//!
//! Reproduced on 14.0.0: `zzzz.parse("x")` gave E_BAD_ARG "zzzz.parse: expected
//! a Record (or a module), got Null", which reads as a type error on a value the
//! reader never made. It is now E_UNKNOWN_BUILTIN "unknown module: zzzz", the
//! code `nope()` gets, with a did-you-mean over the module names. A bound
//! variable that is not a record keeps its record error, and so does a known
//! module's real member. A script echoes its last value, so each one ends in a
//! bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_unkmod_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("module.ish");
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
fn an_unknown_module_is_the_unknown_name_error() {
    let (out, err, code) = run_script("zzzz", "zzzz.parse(\"x\")\n");
    assert_ne!(code, 0, "ran; stdout {out:?}");
    assert!(err.contains("E_UNKNOWN_BUILTIN"), "{err}");
    assert!(err.contains("unknown module: zzzz"), "{err}");
    assert!(
        !err.contains("expected a Record"),
        "still reported as a record error: {err}"
    );
}

#[test]
fn a_misspelled_module_suggests_the_module_name() {
    let (_, err, code) = run_script("fils", "fils.read(\"x\")\n");
    assert_ne!(code, 0);
    assert!(err.contains("did you mean"), "{err}");
    assert!(err.contains("file"), "no module suggested: {err}");
}

#[test]
fn an_unknown_bare_name_call_is_unchanged() {
    let (_, err, code) = run_script("nope", "nope()\n");
    assert_ne!(code, 0);
    assert!(err.contains("E_UNKNOWN_BUILTIN"), "{err}");
    assert!(err.contains("unknown builtin: nope"), "{err}");
}

#[test]
fn a_bound_non_record_keeps_the_record_error() {
    let (_, err, code) = run_script("bound", "x = 5\nx.foo\n");
    assert_ne!(code, 0);
    assert!(err.contains("expected a Record"), "{err}");
}

#[test]
fn a_known_module_member_still_runs() {
    let (out, err, code) = run_script("known", "str.upper(\"abc\")\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "ABC");
}
