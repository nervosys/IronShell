//! The errors for spellings from other languages say what to write instead,
//! run through the real binary. Hints only: the error codes and the messages
//! are unchanged.
//!
//! Reproduced on 14.0.0, each with the generic hint and no fix named:
//! `where(fn(r) => r.state = "open")` was E_PARSE "expected ')'" (`=` assigns, it
//! does not compare); `2 ** 3` was E_PARSE "unexpected token Star"; `contains`
//! given an Array was E_BAD_ARG "expected a string, got Array"; and
//! `"bug" in ["bug","x"]` was E_BAD_ARG about the missing argument. `int("42")`
//! suggests `in`; no string-to-number builtin exists, so its hint is unchanged.
//! A script echoes its last value, so each one ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_hints_{tag}_{}", std::process::id()));
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
fn a_single_equals_in_a_comparison_names_the_comparison() {
    let (_, err, code) = run_script(
        "eq",
        "where([{\"state\": \"open\"}], fn(r) => r.state = \"open\")\n",
    );
    assert_ne!(code, 0);
    assert!(err.contains("E_PARSE"), "{err}");
    assert!(
        err.contains("use `==` to compare; `=` assigns"),
        "no comparison hint: {err}"
    );
}

#[test]
fn a_double_star_names_the_power_function() {
    let (_, err, code) = run_script("pow", "2 ** 3\n");
    assert_ne!(code, 0);
    assert!(err.contains("E_PARSE"), "{err}");
    assert!(
        err.contains("no `**` operator: use math.pow(a, b)"),
        "no power hint: {err}"
    );
}

#[test]
fn contains_given_an_array_names_the_membership_builtin() {
    let (_, err, code) = run_script("contains", "contains([1, 2], 1)\n");
    assert_ne!(code, 0);
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(
        err.contains("contains searches a string for a substring"),
        "{err}"
    );
    assert!(err.contains("`value | in(array)`"), "{err}");
}

#[test]
fn in_without_its_piped_value_shows_the_piped_form() {
    let (_, err, code) = run_script("in", "\"bug\" in [\"bug\", \"x\"]\n");
    assert_ne!(code, 0);
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(
        err.contains("\"bug\" | in([\"bug\", \"x\"])"),
        "no piped form: {err}"
    );
}

#[test]
fn the_piped_membership_form_still_works() {
    let (out, err, code) = run_script("in_ok", "\"bug\" | in([\"bug\", \"x\"])\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "true");
}

#[test]
fn contains_on_a_string_still_works() {
    let (out, err, code) = run_script("contains_ok", "\"hello\" | contains(\"ell\")\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "true");
}
