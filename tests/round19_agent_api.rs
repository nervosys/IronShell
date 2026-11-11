//! Round 19: the machine interface `ish agent execute` (exit status, request
//! hints, arity messages, module-qualified names, retryable), the MCP
//! `ironshell` meta-tool's arguments, and the `sqrt` domain error.
//!
//! Each `ish` test spawns the real binary with a JSON request on stdin, and a
//! session directory in a temp folder, so nothing is written under the
//! repository.

use iron_shell::mcp::McpServer;
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

// ── Item 2: the subject is not counted as a missing parameter ───────────────

#[test]
fn item2_an_array_whose_first_element_is_the_subject_names_the_missing_parameter() {
    let (code, v) = execute_once(r#"{"action":"call","builtin":"pow","args":[2]}"#);
    assert_eq!(code, Some(1), "{v}");
    assert_eq!(v["error"]["code"], json!("E_BAD_ARG"), "{v}");
    let message = v["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("missing required argument 'exponent'"),
        "{message}"
    );
    assert!(
        message.contains("the first argument is the subject"),
        "{message}"
    );
    assert!(!message.contains("0 of 1"), "{message}");
}

// ── Item 3: a module-qualified call name resolves as eval resolves it ───────

#[test]
fn item3_a_module_qualified_call_runs_its_builtin() {
    let (code, v) = execute_once(r#"{"action":"call","builtin":"math.sqrt","args":[16]}"#);
    assert_eq!(code, Some(0), "{v}");
    assert_eq!(v["result"], json!(4.0), "{v}");

    let (code, v) = execute_once(r#"{"action":"call","builtin":"str.upper","args":["abc"]}"#);
    assert_eq!(code, Some(0), "{v}");
    assert_eq!(v["result"], json!("ABC"), "{v}");
}

#[test]
fn item3_an_unknown_module_member_says_to_drop_the_prefix() {
    let (code, v) = execute_once(r#"{"action":"call","builtin":"math.nosuch_zz","args":[1]}"#);
    assert_eq!(code, Some(1), "{v}");
    assert_eq!(v["error"]["code"], json!("E_BAD_ARG"), "{v}");
    let hint = v["error"]["hint"].as_str().unwrap_or_default();
    assert!(hint.contains("drop the module prefix"), "{hint}");
    assert!(hint.contains("nosuch_zz"), "{hint}");
}

// ── Item 5: request hints for a pipeline and for several requests ───────────

#[test]
fn item5_a_pipeline_written_with_stages_gets_the_pipeline_shape_as_its_hint() {
    let (code, v) =
        execute_once(r#"{"action":"pipeline","stages":[{"builtin":"upper","args":["a"]}]}"#);
    assert_eq!(code, Some(1), "{v}");
    assert_eq!(v["error"]["code"], json!("E_BAD_ARG"), "{v}");
    let hint = v["error"]["hint"].as_str().unwrap_or_default();
    assert!(hint.contains(r#""steps":[{"builtin""#), "{hint}");
    assert!(hint.contains("not stages"), "{hint}");
}

#[test]
fn item5_several_requests_on_stdin_say_one_request_per_line_is_interactive() {
    let input = concat!(
        "{\"action\":\"eval\",\"code\":\"1\"}\n",
        "{\"action\":\"eval\",\"code\":\"2\"}\n",
    );
    let (code, out, err) = run_agent(&["execute"], input.as_bytes());
    assert_eq!(code, Some(1), "stdout: {out}; stderr: {err}");
    let v: Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
    assert_eq!(v["error"]["code"], json!("E_PARSE"), "{v}");
    let hint = v["error"]["hint"].as_str().unwrap_or_default();
    assert!(hint.contains("execute takes one request"), "{hint}");
    assert!(hint.contains("ish agent interactive"), "{hint}");
    assert!(hint.contains("actions: call"), "{hint}");
}

// ── Item 4: the MCP `ironshell` meta-tool's arguments ───────────────────────

/// One `tools/call` through the stdio dispatcher, as a JSON-RPC response.
fn mcp_call(name: &str, arguments: Value) -> Value {
    McpServer::new()
        .handle_rpc(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))
        .expect("tools/call has a response")
}

/// The text of a `tools/call` result, which must be a result and not an error.
fn mcp_text(res: &Value) -> String {
    assert!(
        res.get("error").is_none(),
        "unexpected JSON-RPC error: {res}"
    );
    res["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn item4_the_meta_tool_takes_an_args_object_as_named_arguments() {
    let res = mcp_call(
        "ironshell",
        json!({"name": "upper", "args": {"subject": "hi"}}),
    );
    assert_eq!(res["result"]["isError"], false, "{res}");
    assert_eq!(mcp_text(&res), "HI", "{res}");

    let res = mcp_call(
        "ironshell",
        json!({"name": "date_add", "args": {"subject": "2024-02-28", "amount": 1, "unit": "days"}}),
    );
    assert_eq!(mcp_text(&res), "2024-02-29", "{res}");
}

#[test]
fn item4_a_key_beside_args_is_refused_and_the_valid_keys_are_named() {
    let res = mcp_call(
        "ironshell",
        json!({"name": "upper", "args": ["hi"], "bogus": 1}),
    );
    assert_eq!(res["result"]["isError"], true, "{res}");
    let text = mcp_text(&res);
    assert!(
        text.contains("unknown argument 'bogus' for 'ironshell'"),
        "{text}"
    );
    assert!(text.contains("valid arguments: name, args"), "{text}");

    let res = mcp_call("ontology_describe", json!({"args": ["upper"], "bogus": 1}));
    assert_eq!(res["result"]["isError"], true, "{res}");
    let text = mcp_text(&res);
    assert!(
        text.contains("unknown argument 'bogus' for 'ontology_describe'"),
        "{text}"
    );
    assert!(text.contains("valid arguments: args"), "{text}");
}

#[test]
fn item4_a_missing_meta_tool_name_is_a_json_rpc_error_object() {
    let res = mcp_call("ironshell", json!({"args": ["hi"]}));
    assert!(res.get("result").is_none(), "{res}");
    assert_eq!(res["error"]["code"], -32602, "{res}");
    assert!(
        res["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("missing 'name'")),
        "{res}"
    );
}

#[test]
fn item4_the_meta_tool_advertises_args_as_an_array_or_an_object() {
    let tools = McpServer::new().list_builtin_tools();
    let tool = tools
        .iter()
        .find(|t| t.name == "ironshell")
        .expect("the ironshell tool");
    assert_eq!(
        tool.input_schema["properties"]["args"]["type"],
        json!(["array", "object"]),
        "{}",
        tool.input_schema
    );
}
