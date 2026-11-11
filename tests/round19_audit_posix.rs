//! Round 19 audit defects: POSIX-subset refusal naming, the audit-log tamper
//! code, and redaction of the exec audit entry. Each test drives the real binary
//! in agent mode, in a scratch workspace with its own session directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh scratch directory under the system temp dir: `proj` is the workspace,
/// `outside` holds files the jail must not touch, and `session` holds the journal.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ish_round19_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj")).expect("workspace dir");
    std::fs::create_dir_all(dir.join("outside")).expect("outside dir");
    std::fs::create_dir_all(dir.join("session")).expect("session dir");
    dir
}

/// Run `args` in agent mode with `dir/proj` as the workspace.
fn ish_agent(dir: &Path, args: &[&str]) -> Output {
    let ws = dir.join("proj");
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--agent", "--workspace"])
        .arg(&ws)
        .args(args)
        .current_dir(&ws)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_SCRIPT_DEPTH")
        .env_remove("IRONSHELL_AUDIT_LOG")
        .output()
        .expect("run ish")
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// ── Item 1: a script refused as outside the workspace names `sh` ──

/// `sh ../outside/x.sh` is refused before anything runs, and the refusal names
/// the command the script runs as (`sh`), not `cat`, which was never run.
#[cfg(windows)]
#[test]
fn a_script_refusal_names_sh_not_cat() {
    let dir = scratch("sh_name");
    std::fs::write(
        dir.join("outside").join("x.sh"),
        "#!/bin/sh\necho ran > ran.txt\n",
    )
    .unwrap();
    let out = ish_agent(&dir, &["-b", "-c", "sh ../outside/x.sh"]);
    let err = stderr_of(&out);
    assert!(!out.status.success(), "the script must be refused: {err}");
    assert!(err.contains("sh: `"), "the refusal names sh: {err}");
    assert!(err.contains("outside the workspace"), "{err}");
    assert!(
        !err.contains("cat:"),
        "the refusal must not name cat: {err}"
    );
    assert!(
        !dir.join("proj").join("ran.txt").exists(),
        "the refused script must not run"
    );
}
