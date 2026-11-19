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

// ── B2: $((...)) arithmetic ────────────────────────────────────────────────

#[test]
fn b2_arithmetic_expansion_matches_bash() {
    let dir = scratch("b2_arith");
    let cases = [
        ("echo $((7/2)) $((-7/2)) $((-7%2)) $((2**3))", "3 -3 -1 8"),
        ("n=0; n=$((n+1)); n=$((n+1)); echo $n", "2"),
        ("a=6; b=7; echo $((a*b)) $(($a*$b))", "42 42"),
        ("i=2; echo $(( i < 3 )) $(( i > 3 ))", "1 0"),
        ("echo $((0x10 + 010))", "24"),
        ("echo $((5>3 ? 10 : 20))", "10"),
        ("x=5; echo $((x+=3)) $x", "8 8"),
        ("echo $((9223372036854775807 + 1))", "-9223372036854775808"),
        ("for i in 1 2 3; do s=$((s+i)); done; echo $s", "6"),
    ];
    for agent in [false, true] {
        for (script, want) in cases {
            let (out, err, code) = ish_bash(agent, script, &dir);
            assert_eq!((out.as_str(), code), (want, 0), "{script}: {err}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn b2_an_arithmetic_error_names_the_expression_and_exits_1() {
    let dir = scratch("b2_err");
    for (script, text) in [
        ("echo $((1/0))", "division by 0"),
        ("echo $((5%0))", "division by 0"),
        ("echo $((1+))", "operand expected"),
        ("echo $((08))", "value too great"),
    ] {
        let (out, err, code) = ish_bash(false, script, &dir);
        assert_eq!(code, 1, "{script}: {out} {err}");
        assert_eq!(out, "", "{script}: the command must not run");
        assert!(err.contains(text), "{script}: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
