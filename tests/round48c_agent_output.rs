//! Round 48c: agent-mode output and error text.
//!
//! Each test runs the `ish` binary with a private session directory.

use std::path::PathBuf;
use std::process::Command;

fn session(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ish_r48c_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("session dir");
    dir
}

/// Run `ish` with `args`, returning stdout and stderr with CRLF folded to LF.
fn ish(dir: &PathBuf, args: &[&str]) -> (String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env_remove("IRONSHELL_HANDLE_BYTES")
        .env_remove("IRONSHELL_DETERMINISTIC")
        .output()
        .expect("run ish");
    (
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n"),
    )
}

fn agent(dir: &PathBuf, code: &str) -> (String, String) {
    ish(dir, &["--agent", "-c", code])
}

#[test]
fn a_zero_pid_error_says_it_needs_an_id_above_zero() {
    let dir = session("pid0");
    for code in ["proc.info(0)", "proc.exists(0)"] {
        let (out, err) = agent(&dir, code);
        let all = format!("{out}{err}");
        assert!(all.contains("greater than 0"), "{code}: {all}");
        assert!(all.contains("process groups"), "{code}: {all}");
        assert!(!all.contains("addresses groups"), "{code}: {all}");
    }
}

#[test]
fn a_handle_missing_from_this_session_says_so() {
    let dir = session("nohandle");
    let (out, err) = agent(&dir, r#"handle("h1")"#);
    let all = format!("{out}{err}");
    assert!(all.contains("no handles live in this session"), "{all}");
    assert!(all.contains("IRONSHELL_SESSION_DIR="), "{all}");
    assert!(
        all.contains("readable only from the session that made it"),
        "{all}"
    );
    assert!(!all.contains("do not survive the process"), "{all}");
}
