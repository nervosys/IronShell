//! `sort` called directly on its subject, run through the real binary.
//!
//! Reproduced on 14.0.0: `sort([2, 1.5, 3])` failed with E_BAD_ARG "sort:
//! expected a piped Array or String, got a direct call with no subject". The
//! piped form worked. A direct call's first argument is now the subject, so
//! each direct form must print exactly what its piped form prints. Each
//! script ends in a bare expression, because a script echoes its last value.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_sortdirect_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("sort_direct.ish");
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

fn assert_prints(tag: &str, source: &str, expected: &str) {
    let (out, err, code) = run_script(tag, source);
    assert_eq!(code, 0, "source {source:?} failed: {err}");
    assert_eq!(out, expected, "source {source:?}");
}

#[test]
fn sort_of_a_mixed_number_array_directly_matches_the_piped_form() {
    let (piped, err, code) = run_script("piped_num", "[2, 1.5, 3] | sort\n");
    assert_eq!(code, 0, "piped form failed: {err}");
    assert_prints("direct_num", "sort([2, 1.5, 3])\n", &piped);
}

#[test]
fn sort_of_a_number_array_directly_is_in_order() {
    assert_prints("direct_order", "sort([3, 1, 2])\n", "[1, 2, 3]");
}

#[test]
fn sort_of_a_string_array_directly_is_in_order() {
    assert_prints("direct_str", "sort([\"b\", \"a\"])\n", "[\"a\", \"b\"]");
}

#[test]
fn sort_of_a_string_directly_sorts_its_lines_like_the_piped_form() {
    let (piped, err, code) = run_script("piped_lines", "\"b\\na\" | sort\n");
    assert_eq!(code, 0, "piped form failed: {err}");
    assert_prints("direct_lines", "sort(\"b\\na\")\n", &piped);
}

#[test]
fn sort_of_a_mixed_array_directly_fails_like_the_piped_form() {
    let (_, piped_err, piped_code) = run_script("piped_mixed", "[2, \"x\"] | sort\n");
    let (_, direct_err, direct_code) = run_script("direct_mixed", "sort([2, \"x\"])\n");
    assert_ne!(piped_code, 0, "the piped mixed sort must refuse");
    assert_eq!(direct_code, piped_code, "direct: {direct_err}");
    assert!(
        !direct_err.contains("direct call with no subject"),
        "the direct call must reach the sort, not the subject check: {direct_err}"
    );
    assert!(
        direct_err.contains("sort"),
        "the refusal must name sort: {direct_err}"
    );
    assert!(!piped_err.is_empty());
}

#[test]
fn sort_with_no_subject_still_says_it_has_none() {
    let (_, err, code) = run_script("nosubject", "sort()\n");
    assert_ne!(code, 0);
    assert!(
        err.contains("direct call with no subject"),
        "a bare sort() keeps its message: {err}"
    );
}
