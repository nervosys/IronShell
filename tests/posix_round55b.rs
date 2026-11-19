//! Round 55b: the in-process POSIX engine (`ish -b`).
//!
//! Each test runs the `ish` binary in a scratch directory used as the session
//! and workspace root.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r55b_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `ish -b -c <script>` in `dir`; `agent` sets the agent mode for that
/// child only. Returns (stdout, stderr, exit code).
fn ish_bash(agent: bool, script: &str, dir: &Path) -> (String, String, i32) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["-b", "-c", script])
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("state"))
        .env("IRONSHELL_WORKSPACE", dir);
    if agent {
        cmd.env("IRONSHELL_MODE", "agent");
    } else {
        cmd.env_remove("IRONSHELL_MODE");
    }
    let out = cmd.output().expect("run ish");
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

// ── B1: a steering name is refused only where it reaches a child ────────────

#[test]
fn b1_a_plain_assignment_of_a_steering_name_runs_in_agent_mode() {
    let dir = scratch("b1_plain");
    for script in [
        "tmp=w30.tmp; echo $tmp",
        "TMP=w30.tmp; echo $TMP",
        "for TMP in w30.tmp; do echo $TMP; done",
        "echo w30.tmp | read TMP; echo done",
    ] {
        let (out, err, code) = ish_bash(true, script, &dir);
        assert_eq!(code, 0, "`{script}` exited {code}: {out} {err}");
        assert!(!err.contains("E_POLICY_DENY"), "`{script}`: {err}");
        assert!(
            out.contains("w30.tmp") || out.contains("done"),
            "`{script}`: {out}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn b1_export_and_a_prefix_assignment_of_a_steering_name_stay_refused() {
    let dir = scratch("b1_export");
    for script in [
        "export tmp=x",
        "export TMP=x",
        "tmp=x; export tmp",
        "TMP=x git --version",
    ] {
        let (out, err, code) = ish_bash(true, script, &dir);
        assert_eq!(code, 77, "`{script}` exited {code}: {out} {err}");
        assert!(err.contains("E_POLICY_DENY"), "`{script}`: {err}");
    }
    // A human may export it.
    let (out, err, code) = ish_bash(false, "export TMP=x; echo $TMP", &dir);
    assert_eq!((code, out.as_str()), (0, "x"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
