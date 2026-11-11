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
fn a_piped_subject_can_be_named_subject_or_input_in_object_args() {
    let upper = execute(r#"{"action":"call","builtin":"upper","args":{"subject":"abc"}}"#);
    assert_eq!(upper["result"], json!("ABC"), "{upper}");

    let via_input = execute(r#"{"action":"call","builtin":"upper","args":{"input":"abc"}}"#);
    assert_eq!(via_input["result"], json!("ABC"), "{via_input}");

    let added = execute(
        r#"{"action":"call","builtin":"date_add","args":{"subject":"2024-02-28","amount":1,"unit":"days"}}"#,
    );
    assert_eq!(added["result"], json!("2024-02-29"), "{added}");

    let positional = execute(r#"{"action":"call","builtin":"upper","args":["abc"]}"#);
    assert_eq!(positional["result"], json!("ABC"), "{positional}");
}

#[test]
fn a_json_null_argument_is_the_null_value() {
    let v = execute(r#"{"action":"call","builtin":"type_of","args":[null]}"#);
    assert_eq!(v["result"], json!("Null"), "{v}");
}

#[test]
fn schema_takes_the_cli_format_names_and_the_serde_names() {
    for format in [
        "openai",
        "claude",
        "azure_openai",
        "deepseek",
        "vllm",
        "json_schema",
        "open_a_i",
    ] {
        let v = execute(&format!(r#"{{"action":"schema","format":"{format}"}}"#));
        assert_eq!(v["success"], json!(true), "format {format}: {v}");
    }
}

#[test]
fn code_executed_echoes_short_code_whole_and_a_long_program_only_in_part() {
    let short = execute(r#"{"action":"eval","code":"1 + 2"}"#);
    assert_eq!(
        short["metadata"]["code_executed"],
        json!("1 + 2"),
        "{short}"
    );

    let long_code = format!("\"{}\"", "x".repeat(8000));
    let request = json!({"action": "eval", "code": long_code}).to_string();
    let v = execute(&request);
    assert_eq!(v["result"].as_str().map(str::len), Some(8000), "{v}");
    let echoed = v["metadata"]["code_executed"].as_str().expect("echo");
    assert!(echoed.starts_with("\"xxx"), "{echoed}");
    assert!(echoed.ends_with("... (8002 chars)"), "{echoed}");
    assert_eq!(
        echoed.chars().count(),
        200 + "... (8002 chars)".len(),
        "{echoed}"
    );
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
