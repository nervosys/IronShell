//! `--workspace DIR` must name a directory that exists.
//!
//! A nonexistent DIR used to be created, with `.ironshell/audit.log` inside it,
//! so a mistyped path quietly made a new jail. It is now refused before anything
//! runs, with a usage error. These drive the real binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("workspace_must_exist")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    dir
}

fn ish(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_APPROVE")
        .output()
        .expect("run ish")
}

#[test]
fn a_nonexistent_workspace_is_a_usage_error_and_is_not_created() {
    let dir = scratch("missing");
    let missing = dir.join("no-such-ws");
    let out = ish(
        &dir,
        &["--workspace", missing.to_str().unwrap(), "-c", "echo ran"],
    );

    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains(&format!(
            "ish: --workspace {} does not exist",
            missing.display()
        )),
        "{err}"
    );
    assert!(err.contains("hint:"), "the message needs a hint: {err}");
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("ran"),
        "nothing may run: {out:?}"
    );
    assert!(!missing.exists(), "the workspace must not be created");
}

#[test]
fn an_existing_workspace_runs_as_before() {
    let dir = scratch("existing");
    let ws = dir.join("ws");
    std::fs::create_dir_all(&ws).expect("workspace");
    let out = ish(
        &dir,
        &["--workspace", ws.to_str().unwrap(), "-c", "echo ran"],
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ran");
}

#[test]
fn no_workspace_flag_is_unchanged() {
    let dir = scratch("default");
    let out = ish(&dir, &["-c", "echo ran"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ran");
}
