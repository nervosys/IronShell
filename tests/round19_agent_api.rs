//! Round 19: the machine interface `ish agent execute` (exit status, request
//! hints, arity messages, module-qualified names, retryable), the MCP
//! `ironshell` meta-tool's arguments, and the `sqrt` domain error.
//!
//! Each `ish` test spawns the real binary with a JSON request on stdin, and a
//! session directory in a temp folder, so nothing is written under the
//! repository.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh temp directory for one spawned process.
fn scratch_dir() -> std::path::PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("ae_r19_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run `ish agent <args>` with `stdin` as its input. Returns the exit code,
/// stdout and stderr.
fn run_agent(args: &[&str], stdin: &[u8]) -> (Option<i32>, String, String) {
    let mut full = vec!["agent"];
    full.extend_from_slice(args);
    run_ish(&full, stdin)
}

/// Run `ish <args>` with `stdin` as its input. Returns the exit code, stdout
/// and stderr.
fn run_ish(args: &[&str], stdin: &[u8]) -> (Option<i32>, String, String) {
    let dir = scratch_dir();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .env("IRONSHELL_SESSION_DIR", &dir)
        .env_remove("IRONSHELL_DETERMINISTIC")
        .env_remove("IRONSHELL_TOKEN_BUDGET")
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ish");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait for ish");
    let _ = std::fs::remove_dir_all(&dir);
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Run `ish agent execute` with one request. Returns the exit code and stdout
/// parsed as JSON.
fn execute_once(request: &str) -> (Option<i32>, Value) {
    let (code, out, err) = run_agent(&["execute"], request.as_bytes());
    let v = serde_json::from_str(&out)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {out}; stderr: {err}"));
    (code, v)
}

// ── Item 1: a failed call exits 1 ───────────────────────────────────────────

#[test]
fn item1_a_failed_call_exits_1_and_still_prints_its_response() {
    for request in [
        r#"{"action":"describe","builtin":"no_such_builtin_zz"}"#,
        r#"{"action":"eval","code":"((("}"#,
        r#"{"action":"call","builtin":"upper","args":{"bogus":1}}"#,
    ] {
        let (code, v) = execute_once(request);
        assert_eq!(code, Some(1), "{request}: {v}");
        assert_eq!(v["success"], json!(false), "{request}: {v}");
    }
}

#[test]
fn item1_a_successful_call_still_exits_0() {
    let (code, v) = execute_once(r#"{"action":"eval","code":"1 + 1"}"#);
    assert_eq!(code, Some(0), "{v}");
    assert_eq!(v["result"], json!(2), "{v}");
}
