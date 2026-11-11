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

/// Run `ish agent execute` on raw stdin bytes. A malformed request is answered
/// on stdout as JSON and exits 1; nothing goes to stderr.
fn malformed(input: &[u8]) -> Value {
    let (code, out, err) = run_agent(&["execute"], input);
    assert_eq!(code, Some(1), "exit status; stdout: {out}; stderr: {err}");
    assert!(err.is_empty(), "stderr must stay empty: {err}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {out}"))
}

#[test]
fn every_malformed_request_gets_one_json_error_and_exit_status_1() {
    let shape_cases: [&str; 8] = [
        "{}",
        r#"{"builtin":"len","args":[]}"#,
        r#"{"action":"frobnicate"}"#,
        r#"{"action":"call","builtin":42}"#,
        "[1, 2]",
        "null",
        r#"{"action":"call","builtin":"len","args":[]} trailing"#,
        "",
    ];
    for case in shape_cases {
        let v = malformed(case.as_bytes());
        assert_eq!(v["success"], json!(false), "{case}: {v}");
        assert!(v["error"]["code"].is_string(), "{case}: {v}");
        assert!(v["error"]["message"].is_string(), "{case}: {v}");
        assert!(
            v["error"]["hint"]
                .as_str()
                .is_some_and(|h| h.contains("actions: call")),
            "{case}: {v}"
        );
    }
    let parse = malformed(br#"{"action":"call","builtin":"len","args":[]} trailing"#);
    assert_eq!(parse["error"]["code"], json!("E_PARSE"), "{parse}");
    let shape = malformed(b"{}");
    assert_eq!(shape["error"]["code"], json!("E_BAD_ARG"), "{shape}");

    let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    let v = malformed(deep.as_bytes());
    assert_eq!(v["error"]["code"], json!("E_PARSE"), "{v}");
}

#[test]
fn a_leading_byte_order_mark_is_ignored() {
    let request = "\u{feff}{\"action\":\"eval\",\"code\":\"1 + 1\"}";
    let v = execute(request);
    assert_eq!(v["result"], json!(2), "{v}");
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

/// A failure's `error` is an object with `code`, `message`, `hint` and
/// `retryable`, whichever path produced it.
fn assert_error_object(v: &Value, code: &str) {
    assert_eq!(v["success"], json!(false), "{v}");
    assert!(v["error"].is_object(), "error is not an object: {v}");
    assert_eq!(v["error"]["code"], json!(code), "{v}");
    assert!(v["error"]["message"].is_string(), "{v}");
    assert!(
        v["error"]["hint"].as_str().is_some_and(|h| !h.is_empty()),
        "{v}"
    );
    assert!(v["error"]["retryable"].is_boolean(), "{v}");
}

#[test]
fn every_failure_has_an_error_object_with_code_message_hint_and_retryable() {
    let describe = execute(r#"{"action":"describe","builtin":"no_such_builtin_zz"}"#);
    assert_error_object(&describe, "E_UNKNOWN_BUILTIN");
    assert_eq!(describe["code"], json!("E_UNKNOWN_BUILTIN"), "{describe}");

    let convert = execute(r#"{"action":"call","builtin":"upper","args":{"bogus":1}}"#);
    assert_error_object(&convert, "E_BAD_ARG");
    assert!(
        convert["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("Failed to convert arguments")),
        "{convert}"
    );

    let parse = execute(r#"{"action":"eval","code":"((("}"#);
    assert_error_object(&parse, "E_PARSE");
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
