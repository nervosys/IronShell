//! `contains` names its three subject types when the subject is none of them,
//! and `arr.contains` is the Array form of the same builtin, as `str.contains`
//! is the String form.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_contains_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("contains.ish");
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
fn an_int_subject_is_refused_naming_all_three_subject_types() {
    let (_, err, code) = run_script("int_subject", "contains(5, 5)\n");
    assert_ne!(code, 0);
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(
        err.contains("contains: expected a String, Array or Record, got Int"),
        "{err}"
    );
}

#[test]
fn arr_contains_finds_an_element_as_contains_does() {
    let (out, err, code) = run_script(
        "arr",
        "arr.contains([1, 2, 3], 2)\narr.contains([1, 2, 3], 9)\n",
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "false", "a script echoes only its last value");
}

#[test]
fn arr_contains_matches_the_array_form_of_contains() {
    let (out, err, code) = run_script("arr_eq", "[1, 2, 3] | arr.contains(2)\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "true");
}
