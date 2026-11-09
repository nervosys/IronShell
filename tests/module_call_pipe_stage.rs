//! `module.member(args)` works as a pipeline stage, run through the real binary.
//!
//! Reproduced on 14.0.0: `"abc" | str.upper()` and `"abc" | crypto.hash("sha256")`
//! were E_PARSE "unexpected token Dot", because a pipe stage was parsed as a bare
//! name and a call. The underscore builtin spelling of the same stage works, so
//! the module spelling now parses the same way. Without a call (`"abc" | str.upper`)
//! it is the same stage as `str.upper()`. A script echoes its last value, so each
//! one ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_modpipe_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("pipe.ish");
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
fn a_module_call_with_no_arguments_is_a_pipe_stage() {
    assert_prints("upper", "\"abc\" | str.upper()\n", "ABC");
}

#[test]
fn a_module_call_with_an_argument_is_a_pipe_stage() {
    assert_prints(
        "hash",
        "\"abc\" | crypto.hash(\"sha256\")\n",
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
}

#[test]
fn a_module_call_takes_the_piped_value_before_its_arguments() {
    assert_prints("contains", "\"abc\" | str.contains(\"b\")\n", "true");
}

#[test]
fn a_module_call_stage_can_be_followed_by_another_stage() {
    assert_prints("chain", "\"abc\" | str.upper() | str.len()\n", "3");
}

#[test]
fn the_underscore_builtin_stage_it_matches_still_works() {
    assert_prints("bare", "\"abc\" | upper()\n", "ABC");
}

#[test]
fn a_module_member_without_a_call_is_a_pipe_stage() {
    assert_prints("nocall", "\"abc\" | str.upper\n", "ABC");
}

#[test]
fn an_unknown_module_as_a_pipe_stage_is_the_unknown_name_error() {
    let (_, err, code) = run_script("unknown", "\"abc\" | nope.thing()\n");
    assert_ne!(code, 0);
    assert!(err.contains("unknown module: nope"), "{err}");
}
