//! When a script holds several constructs the subset refuses, the refusal names
//! the first one by position. The lexer used to refuse a `&` before the parser
//! saw a subshell on an earlier line, so the later construct was named.

use std::process::{Command, Output};

fn run(tag: &str, script: &str) -> Output {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("posix_refusal_first")
        .join(tag);
    let _ = std::fs::remove_dir_all(&root);
    let session = root.join("session");
    std::fs::create_dir_all(&session).expect("scratch dir");
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(&root)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", &session)
        .env_remove("IRONSHELL_ALLOW_SH")
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish")
}

#[test]
fn an_earlier_subshell_is_named_before_a_later_background_job() {
    let out = run("subshell_first", "echo one\n(echo two)\necho three &\n");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(77), "{err}");
    assert!(out.stdout.is_empty(), "ran something");
    assert!(
        err.contains("subshell at line 2, column 1 is not in the in-process POSIX subset"),
        "{err}"
    );
}

#[test]
fn an_earlier_background_job_is_named_before_a_later_subshell() {
    let out = run("background_first", "echo one &\n(echo two)\n");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(77), "{err}");
    assert!(out.stdout.is_empty(), "ran something");
    assert!(
        err.contains("background job at line 1, column 10 is not in the in-process POSIX subset"),
        "{err}"
    );
}
