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

// ── Item 2: a guarded write to the audit log is a protected-evidence refusal ──

/// `.ironshell/audit.log` is inside the workspace, so the refusal is a policy
/// decision (E_POLICY_DENY), not a containment failure (E_OUTSIDE_WORKSPACE).
#[test]
fn a_write_to_the_audit_log_is_refused_as_protected_policy() {
    let dir = scratch("audit_tamper");
    std::fs::create_dir_all(dir.join("proj").join(".ironshell")).unwrap();
    let out = ish_agent(&dir, &["-c", r#"file_append(".ironshell/audit.log", "x")"#]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        stderr_of(&out)
    );
    assert!(!out.status.success(), "the write must be refused: {text}");
    assert!(text.contains("E_POLICY_DENY"), "{text}");
    assert!(!text.contains("E_OUTSIDE_WORKSPACE"), "{text}");
    assert!(text.contains("protected"), "the refusal says why: {text}");
}

// ── Item 3: the audit entry of a refused exec redacts secret shapes ──

/// An external command refused for approval is audited with its command line.
/// The secret shapes in that line are redacted before the entry is written, and
/// the program name stays. No approval is given, so the Exec gate refuses before
/// the program starts and nothing runs.
#[test]
fn a_refused_exec_audit_entry_redacts_secret_shapes() {
    let dir = scratch("exec_redact");
    std::fs::create_dir_all(dir.join("proj").join(".ironshell")).unwrap();
    let out = ish_agent(
        &dir,
        &[
            "-b",
            "-c",
            "rustc sk-abcdefghijklmnopqrstuvwx99 API_KEY=hunter2hunter2",
        ],
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        stderr_of(&out)
    );
    assert!(!out.status.success(), "the exec must be refused: {text}");
    let log = std::fs::read_to_string(dir.join("proj").join(".ironshell").join("audit.log"))
        .expect("the refusal is audited");
    assert!(log.contains("needs_approval"), "{log}");
    assert!(log.contains("rustc"), "the program name stays: {log}");
    assert!(!log.contains("sk-abcdefghijklmnopqrstuvwx99"), "{log}");
    assert!(!log.contains("hunter2hunter2"), "{log}");
}
