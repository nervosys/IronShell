//! Round 46c: the Agent API server's relative paths, tool names, WebSocket
//! replies, error shapes and listing filters.
//!
//! The HTTP tests drive the real router over a real TCP socket, as
//! `agent_api_auth.rs` does, so the middleware is exercised as mounted. Every
//! server is started from a working directory outside its workspace, which is
//! the situation the relative-path test is about.

#![cfg(feature = "native")]

use iron_shell::agent_api::server::{
    bearer_token, error_for_status, parse_ws_text, start_agent_api_server, stream_eval_events,
    truncate_echo, AgentApiConfig, WsClientMessage, WsServerMessage,
};
use iron_shell::agent_api::{
    canonical_tool_name, parse_schema_format, process_request, unknown_builtin_refusal,
    AgentRequest,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;

const TOKEN: &str = "round46c-token";

/// The scratch directory: the server's start directory. The workspace is `ws`
/// inside it, so a server started here starts outside its workspace.
fn scratch() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("ironshell_r46c_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("ws")).expect("create the workspace");
        std::fs::create_dir_all(dir.join("session")).expect("create the session dir");
        let dir = dir.canonicalize().expect("canonical scratch dir");
        std::env::set_var("IRONSHELL_MODE", "agent");
        std::env::set_var("IRONSHELL_WORKSPACE", dir.join("ws"));
        std::env::set_var("IRONSHELL_SESSION_DIR", dir.join("session"));
        dir
    })
}

/// Serializes "go to the scratch directory, then start a server", so every
/// server starts outside its workspace whatever the other tests are doing.
static START: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn serve() -> u16 {
    let dir = scratch().clone();
    let _held = START.lock().await;
    std::env::set_current_dir(&dir).expect("start outside the workspace");

    // The probed port can be taken again before the server binds it, so a
    // server that stops without listening is retried on a fresh port.
    for _attempt in 0..5 {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let port = probe.local_addr().expect("addr").port();
        drop(probe);
        let config = AgentApiConfig {
            host: "127.0.0.1".to_string(),
            port,
            enable_cors: true,
            auth_token: Some(TOKEN.to_string()),
            request_timeout_secs: 300,
        };
        let server = tokio::spawn(async move {
            if let Err(e) = start_agent_api_server(config).await {
                eprintln!("agent api server on port {port} stopped: {e}");
            }
        });
        // 30 s: a loaded macOS runner can take several seconds to build the
        // ontology before it listens.
        for _ in 0..600 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                return port;
            }
            if server.is_finished() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        if !server.is_finished() {
            panic!("agent api server did not start on port {port}");
        }
    }
    panic!("agent api server stopped before listening on five ports");
}

fn client() -> reqwest::Client {
    iron_shell::tls::install_provider();
    reqwest::Client::new()
}

fn url(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{port}{path}")
}

/// Status, content type and JSON body of a response. Fails when the body is
/// not JSON, which is the point of several tests.
async fn json_of(res: reqwest::Response) -> (u16, Value) {
    let status = res.status().as_u16();
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = res.text().await.expect("body");
    assert!(
        content_type.starts_with("application/json"),
        "status {status} answered {content_type:?}, not JSON: {text:.200}"
    );
    let body =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text:.200}"));
    (status, body)
}

async fn eval(port: u16, code: &str) -> Value {
    let res = client()
        .post(url(port, "/api/v1/eval"))
        .bearer_auth(TOKEN)
        .json(&json!({ "code": code }))
        .send()
        .await
        .expect("request");
    json_of(res).await.1
}

/// The error shape every failure has: `success: false` and an `error` object.
fn assert_error_shape(body: &Value, code: &str) {
    assert_eq!(body["success"], json!(false), "{body}");
    assert!(body["error"].is_object(), "error must be an object: {body}");
    assert_eq!(body["error"]["code"], json!(code), "{body}");
    assert!(body["error"]["message"].is_string(), "{body}");
    assert!(body["error"]["hint"].is_string(), "{body}");
    assert!(body["error"]["retryable"].is_boolean(), "{body}");
}

// ── 1. relative paths, started outside the workspace ───────────────────────

#[tokio::test]
async fn a_relative_path_written_is_read_back_when_started_outside_the_workspace() {
    let port = serve().await;
    let ws = scratch().join("ws");

    let wrote = eval(port, "file.write('b.txt', 'hi')").await;
    assert_eq!(wrote["success"], json!(true), "{wrote}");
    assert!(
        ws.join("b.txt").is_file(),
        "the write must land in the workspace"
    );
    assert!(
        !scratch().join("b.txt").exists(),
        "and not beside the workspace"
    );

    let read = eval(port, "file.read('b.txt')").await;
    assert_eq!(read["success"], json!(true), "{read}");
    assert_eq!(read["result"], json!("hi"), "{read}");

    let cat = eval(port, "cat('b.txt')").await;
    assert_eq!(cat["success"], json!(true), "{cat}");

    let listing = eval(port, "ls('.') | map(fn(f) => f.name)").await;
    assert_eq!(listing["success"], json!(true), "{listing}");
    assert!(listing["result"].to_string().contains("b.txt"), "{listing}");

    // The effect report and the call itself name the same file.
    let explained = eval(port, "explain_effects(\"rm('b.txt')\")").await;
    assert!(
        !explained
            .to_string()
            .to_ascii_lowercase()
            .contains("outside the workspace"),
        "{explained}"
    );
    // Agent mode gates a delete: the call is refused for approval, and the
    // target it names is the file in the workspace, not one beside it.
    let removed = eval(port, "rm('b.txt')").await;
    assert_eq!(removed["success"], json!(false), "{removed}");
    assert_eq!(removed["code"], json!("E_NEEDS_APPROVAL"), "{removed}");
    let target = removed["error"]["approval"]["targets"][0]
        .as_str()
        .unwrap_or_default()
        .replace('\\', "/");
    let expected = ws.join("b.txt").to_string_lossy().replace('\\', "/");
    assert!(
        target.ends_with("/ws/b.txt") && target.contains("ironshell_r46c_"),
        "approval target {target:?}, expected {expected:?}: {removed}"
    );
    assert!(ws.join("b.txt").exists(), "nothing was deleted");
}

// ── 2. the tool names a schema advertises are accepted ─────────────────────

const FORMATS: &[&str] = &[
    "openai",
    "claude",
    "gemini",
    "llama",
    "mistral",
    "cohere",
    "grok",
    "deepseek",
    "bedrock",
    "azure_openai",
    "qwen",
    "ollama",
    "lmstudio",
    "vllm",
    "huggingface",
    "openrouter",
    "kimi",
    "yi",
    "glm",
    "reka",
    "ai21",
    "perplexity",
    "together",
    "groq",
    "fireworks",
];

fn tool_name(item: &Value) -> Option<String> {
    [
        item.get("name"),
        item.get("function").and_then(|f| f.get("name")),
        item.get("toolSpec").and_then(|f| f.get("name")),
    ]
    .into_iter()
    .flatten()
    .find_map(|n| n.as_str().map(str::to_string))
}

fn collect_tool_names(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                if matches!(key.as_str(), "tools" | "function_declarations") {
                    if let Some(items) = v.as_array() {
                        out.extend(items.iter().filter_map(tool_name));
                    }
                }
                collect_tool_names(v, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_tool_names(v, out)),
        _ => {}
    }
}

#[test]
fn every_tool_name_a_schema_format_advertises_is_accepted_by_call() {
    scratch();
    for format in FORMATS {
        let parsed = parse_schema_format(format).unwrap_or_else(|| panic!("format {format}"));
        let response = process_request(&AgentRequest::Schema {
            format: parsed,
            module: Some("math".to_string()),
            q: None,
        });
        assert!(response.success, "{format}: {:?}", response.error);
        let mut names = Vec::new();
        collect_tool_names(response.result.as_ref().expect("schema"), &mut names);
        if matches!(
            *format,
            "openai" | "claude" | "gemini" | "llama" | "mistral"
        ) {
            assert!(!names.is_empty(), "{format} advertised no tools");
        }
        for name in names {
            let builtin = canonical_tool_name(&name);
            assert!(
                unknown_builtin_refusal(&builtin).is_none(),
                "{format}: tool {name:?} resolves to {builtin:?}, which is no builtin"
            );
        }
    }
}

#[test]
fn the_schema_prefix_is_stripped_only_from_a_name_that_is_no_builtin() {
    assert_eq!(canonical_tool_name("ironshell_sqrt"), "sqrt");
    assert_eq!(canonical_tool_name("sqrt"), "sqrt");
    assert_eq!(
        canonical_tool_name("no_such_builtin_zz"),
        "no_such_builtin_zz"
    );
    assert_eq!(canonical_tool_name("ironshell_"), "ironshell_");
}

#[tokio::test]
async fn call_and_execute_and_pipeline_take_the_advertised_tool_names() {
    let port = serve().await;

    let res = client()
        .post(url(port, "/api/v1/call/ironshell_sqrt"))
        .bearer_auth(TOKEN)
        .json(&json!([16]))
        .send()
        .await
        .expect("request");
    let (status, body) = json_of(res).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["success"], json!(true), "{body}");

    let res = client()
        .post(url(port, "/api/v1/execute"))
        .bearer_auth(TOKEN)
        .json(&json!({"action": "call", "builtin": "ironshell_sqrt", "args": [16]}))
        .send()
        .await
        .expect("request");
    let (status, body) = json_of(res).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["success"], json!(true), "{body}");

    // A pipeline step takes a module-qualified name, as /call does.
    let res = client()
        .post(url(port, "/api/v1/pipeline"))
        .bearer_auth(TOKEN)
        .json(&json!({"steps": [{"builtin": "arr.range", "args": [3]}]}))
        .send()
        .await
        .expect("request");
    let (status, body) = json_of(res).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["result"].as_array().map(Vec::len), Some(3), "{body}");
}

// ── 3. the WebSocket never stays silent ────────────────────────────────────

#[test]
fn a_bare_agent_request_on_the_websocket_is_an_execute() {
    match parse_ws_text(r#"{"action":"eval","code":"1+2"}"#) {
        Ok(WsClientMessage::Execute { id, request }) => {
            assert_eq!(id, "");
            assert!(matches!(request, AgentRequest::Eval { .. }));
        }
        other => panic!("{other:?}"),
    }
    match parse_ws_text(r#"{"id":"q7","action":"eval","code":"1+2"}"#) {
        Ok(WsClientMessage::Execute { id, .. }) => assert_eq!(id, "q7"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        parse_ws_text(r#"{"type":"ping"}"#),
        Ok(WsClientMessage::Ping { .. })
    ));
}

#[test]
fn an_unrecognised_websocket_frame_is_answered_with_an_error_naming_the_shapes() {
    for frame in [
        "not json",
        "{}",
        r#"{"hello":"world"}"#,
        r#"{"type":"nonsense"}"#,
        r#"{"action":"eval"}"#,
    ] {
        match parse_ws_text(frame) {
            Err(WsServerMessage::Error { message, .. }) => {
                assert!(
                    message.contains("\"action\":\"eval\""),
                    "{frame}: {message}"
                );
                assert!(message.contains("\"type\":\"ping\""), "{frame}: {message}");
            }
            other => panic!("{frame}: {other:?}"),
        }
    }
}

// ── 4. every failure has one shape ─────────────────────────────────────────

#[tokio::test]
async fn http_layer_failures_are_json_in_the_api_error_shape() {
    let port = serve().await;
    let c = client();

    let (status, body) = json_of(
        c.get(url(port, "/api/v1/no/such/route"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .expect("request"),
    )
    .await;
    assert_eq!(status, 404);
    assert_error_shape(&body, "E_NOT_FOUND");

    let (status, body) = json_of(
        c.get(url(port, "/api/v1/eval"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .expect("request"),
    )
    .await;
    assert_eq!(status, 405);
    assert_error_shape(&body, "E_BAD_ARG");

    let (status, body) = json_of(
        c.post(url(port, "/api/v1/eval"))
            .json(&json!({"code": "1"}))
            .send()
            .await
            .expect("request"),
    )
    .await;
    assert_eq!(status, 401);
    assert_error_shape(&body, "E_UNAUTHORIZED");

    // Malformed JSON: a coded error, and the echo is cut.
    let (status, body) = json_of(
        c.post(url(port, "/api/v1/eval"))
            .bearer_auth(TOKEN)
            .header("content-type", "application/json")
            .body("{nope".repeat(8_000))
            .send()
            .await
            .expect("request"),
    )
    .await;
    assert_eq!(status, 400);
    assert_error_shape(&body, "E_PARSE");
    assert!(body.to_string().chars().count() < 2_000, "{body:.300}");

    let (status, body) = json_of(
        c.post(url(port, "/api/v1/eval"))
            .bearer_auth(TOKEN)
            .header("content-type", "text/plain")
            .body("1 + 1")
            .send()
            .await
            .expect("request"),
    )
    .await;
    assert_eq!(status, 415);
    assert_error_shape(&body, "E_BAD_ARG");

    // The server may answer 413 and close before the client finishes
    // sending; Windows then reports the reset instead of the answer.
    match c
        .post(url(port, "/api/v1/eval"))
        .bearer_auth(TOKEN)
        .header("content-type", "application/json")
        .body(" ".repeat(3 * 1024 * 1024))
        .send()
        .await
    {
        Ok(resp) => {
            let (status, body) = json_of(resp).await;
            assert_eq!(status, 413);
            assert_error_shape(&body, "E_BUDGET_EXCEEDED");
        }
        Err(e) => assert!(e.is_request(), "oversized body: {e:?}"),
    }

    let (status, body) = json_of(
        c.get(url(port, "/api/v1/schema/bogus"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .expect("request"),
    )
    .await;
    assert_eq!(status, 400);
    assert_error_shape(&body, "E_BAD_ARG");
}

#[test]
fn a_framework_failure_maps_to_a_code_and_cuts_the_echo() {
    for (status, code) in [
        (400, "E_BAD_ARG"),
        (404, "E_NOT_FOUND"),
        (405, "E_BAD_ARG"),
        (408, "E_BUDGET_EXCEEDED"),
        (413, "E_BUDGET_EXCEEDED"),
        (415, "E_BAD_ARG"),
        (500, "E_UNKNOWN"),
    ] {
        let r = error_for_status(status, "");
        assert_eq!(r.code.as_deref(), Some(code), "{status}");
        assert!(r.hint.is_some(), "{status}");
    }
    let long = "x".repeat(100_000);
    let r = error_for_status(
        400,
        &format!("Failed to parse the request body as JSON: {long}"),
    );
    assert_eq!(r.code.as_deref(), Some("E_PARSE"));
    assert!(r.error.unwrap().chars().count() < 400);
    assert!(truncate_echo(&long, 200).chars().count() < 240);
    assert_eq!(truncate_echo("short", 200), "short");
}

#[tokio::test]
async fn call_with_no_body_and_no_content_type_takes_empty_arguments() {
    let port = serve().await;
    let res = client()
        .post(url(port, "/api/v1/call/now_iso"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .expect("request");
    let (status, body) = json_of(res).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["success"], json!(true), "{body}");

    // A body that is not JSON is a coded parse error, not a 415.
    let res = client()
        .post(url(port, "/api/v1/call/now_iso"))
        .bearer_auth(TOKEN)
        .body("{nope")
        .send()
        .await
        .expect("request");
    let (status, body) = json_of(res).await;
    assert_eq!(status, 400);
    assert_error_shape(&body, "E_PARSE");
}

#[tokio::test]
async fn pipeline_failures_carry_a_real_code() {
    let port = serve().await;
    let c = client();

    let (status, body) = json_of(
        c.post(url(port, "/api/v1/pipeline"))
            .bearer_auth(TOKEN)
            .json(&json!({"steps": [{"builtin": "no_such_builtin_zz"}]}))
            .send()
            .await
            .expect("request"),
    )
    .await;
    assert_eq!(status, 400);
    assert_ne!(body["error"]["code"], json!("E_UNKNOWN"), "{body}");
    assert_eq!(body["error"]["code"], json!("E_UNKNOWN_BUILTIN"), "{body}");

    let (status, body) = json_of(
        c.post(url(port, "/api/v1/pipeline"))
            .bearer_auth(TOKEN)
            .json(&json!({"steps": "not a list"}))
            .send()
            .await
            .expect("request"),
    )
    .await;
    assert_eq!(status, 400);
    assert_error_shape(&body, "E_BAD_ARG");

    let (_, body) = json_of(
        c.post(url(port, "/api/v1/pipeline"))
            .bearer_auth(TOKEN)
            .json(&json!({"steps": []}))
            .send()
            .await
            .expect("request"),
    )
    .await;
    assert_error_shape(&body, "E_BAD_ARG");
}

// ── 5. SSE errors are objects ──────────────────────────────────────────────

#[test]
fn a_streamed_error_event_carries_an_error_object() {
    let events = stream_eval_events("no_such_builtin_zz(1)", None);
    let last = events.last().expect("an event");
    assert_eq!(last.event, "error");
    let error = &last.data["error"];
    assert!(error.is_object(), "{error}");
    assert_eq!(error["code"], json!("E_UNKNOWN_BUILTIN"), "{error}");
    assert!(error["message"].is_string(), "{error}");
    assert!(
        !error["message"].as_str().unwrap().starts_with('{'),
        "the message must not be escaped JSON: {error}"
    );
}

// ── 6. smaller items ───────────────────────────────────────────────────────

#[tokio::test]
async fn builtins_category_accepts_module_names_and_refuses_unknown_ones() {
    let port = serve().await;
    let c = client();
    for category in ["file", "sys", "math", "FileSystem"] {
        let (status, body) = json_of(
            c.get(url(port, &format!("/api/v1/builtins?category={category}")))
                .bearer_auth(TOKEN)
                .send()
                .await
                .expect("request"),
        )
        .await;
        assert_eq!(status, 200, "{category}: {body}");
        assert!(
            body["result"]["count"].as_u64().unwrap_or(0) > 0,
            "{category} listed nothing: {body:.300}"
        );
    }
    let (status, body) = json_of(
        c.get(url(port, "/api/v1/builtins?category=zzz"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .expect("request"),
    )
    .await;
    assert_eq!(status, 400);
    assert_error_shape(&body, "E_BAD_ARG");

    let (_, body) = json_of(
        c.post(url(port, "/api/v1/execute"))
            .bearer_auth(TOKEN)
            .json(&json!({"action": "list_builtins", "category": "zzz"}))
            .send()
            .await
            .expect("request"),
    )
    .await;
    assert_error_shape(&body, "E_BAD_ARG");
}

#[tokio::test]
async fn the_authorization_scheme_is_case_insensitive() {
    assert_eq!(bearer_token("Bearer abc"), Some("abc"));
    assert_eq!(bearer_token("bearer abc"), Some("abc"));
    assert_eq!(bearer_token("BEARER abc"), Some("abc"));
    assert_eq!(bearer_token("Basic abc"), None);
    assert_eq!(bearer_token("Bearer"), None);

    let port = serve().await;
    for scheme in ["bearer", "BEARER", "Bearer"] {
        let res = client()
            .post(url(port, "/api/v1/eval"))
            .header("authorization", format!("{scheme} {TOKEN}"))
            .json(&json!({"code": "1 + 1"}))
            .send()
            .await
            .expect("request");
        assert_eq!(res.status().as_u16(), 200, "{scheme}");
    }
    let res = client()
        .post(url(port, "/api/v1/eval"))
        .header("authorization", format!("bearer {}x", TOKEN))
        .json(&json!({"code": "1 + 1"}))
        .send()
        .await
        .expect("request");
    assert_eq!(res.status().as_u16(), 401);
}
