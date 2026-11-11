//! Round 18: the machine interface `ish agent execute` and `ish agent interactive`.
//!
//! Each test spawns the real binary with a JSON request on stdin, and a session
//! directory in a temp folder, so nothing is written under the repository.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh temp directory for one spawned process.
fn scratch_dir() -> std::path::PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("ae_r18_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run `ish agent <args>` with `stdin` as its input. Returns the exit code,
/// stdout and stderr.
fn run_agent(args: &[&str], stdin: &[u8]) -> (Option<i32>, String, String) {
    let dir = scratch_dir();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg("agent")
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

/// Run `ish agent execute` with one request, and parse its stdout as JSON.
fn execute(request: &str) -> Value {
    let (code, out, err) = run_agent(&["execute"], request.as_bytes());
    assert_eq!(code, Some(0), "stderr: {err}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {out}"))
}

#[test]
fn an_array_result_is_a_json_array_not_text() {
    let v = execute(r#"{"action":"call","builtin":"split","args":["a,b,c",","]}"#);
    assert_eq!(v["success"], json!(true), "{v}");
    assert_eq!(v["result"], json!(["a", "b", "c"]), "{v}");
    assert_eq!(v["result_type"], json!("Array"), "{v}");
}

#[test]
fn a_record_result_is_a_json_object_and_scalars_keep_their_types() {
    let v = execute(r#"{"action":"eval","code":"{x: 1, y: \"two\"}"}"#);
    assert_eq!(v["result"], json!({"x": 1, "y": "two"}), "{v}");
    assert_eq!(v["result_type"], json!("Record"), "{v}");

    let n = execute(r#"{"action":"eval","code":"1 + 2"}"#);
    assert_eq!(n["result"], json!(3), "{n}");
    assert_eq!(n["result_type"], json!("Int"), "{n}");
}
