//! `exit(code)` is allowed in agent mode.
//!
//! It is a process-lifecycle call, and agent mode had been asking for approval
//! before it. What it ends is the ish process alone, so the approval gate is no
//! protection, and an agent that cannot stop its own shell with a code cannot
//! finish a script cleanly. The code is still the exit status.
//!
//! These drive the real binary, because the question is what the process does.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("exit_agent_mode")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    dir
}

/// Run `ish` with `args` under agent mode. Returns (status, stdout, stderr).
fn agent_run(dir: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env("IRONSHELL_ALLOW_SH", "")
        .output()
        .expect("run ish");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn exit_three_under_agent_mode_exits_three() {
    let dir = scratch("three");
    let (status, out, err) = agent_run(&dir, &["--agent", "-c", "exit(3)"]);
    assert_eq!(status, Some(3), "stdout: {out}\nstderr: {err}");
    assert!(!out.contains("E_NEEDS_APPROVAL") && !err.contains("E_NEEDS_APPROVAL"));
}

#[test]
fn exit_under_the_agent_mode_variable_exits_with_its_code() {
    let dir = scratch("env");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", "exit(7)"])
        .current_dir(&dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("IRONSHELL_MODE", "agent")
        .env("NO_COLOR", "1")
        .output()
        .expect("run ish");
    assert_eq!(out.status.code(), Some(7));
    let text =
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    assert!(!text.contains("E_NEEDS_APPROVAL"), "{text}");
}

#[test]
fn exit_with_no_code_exits_zero_under_agent_mode() {
    let dir = scratch("zero");
    let (status, out, err) = agent_run(&dir, &["--agent", "-c", "exit()"]);
    assert_eq!(status, Some(0), "stdout: {out}\nstderr: {err}");
}

#[test]
fn an_out_of_range_code_is_still_refused_under_agent_mode() {
    let dir = scratch("range");
    let (status, _, err) = agent_run(&dir, &["--agent", "-c", "exit(256)"]);
    assert_ne!(status, Some(0), "{err}");
    assert!(
        err.contains("E_BAD_ARG") || err.contains("0..=255"),
        "{err}"
    );
}
