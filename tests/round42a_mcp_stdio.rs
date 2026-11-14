//! `ish mcp stdio`: module-qualified names, malformed messages, and result text.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

fn rpc(req: Value) -> Option<Value> {
    iron_shell::mcp::McpServer::new().handle_rpc(&req)
}

fn call(name: &str, arguments: Value) -> Value {
    rpc(json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    }))
    .expect("a response")
}

fn text_of(res: &Value) -> String {
    res["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

// 1
#[test]
fn a_module_qualified_name_runs_its_builtin_through_the_meta_tool() {
    let res = call("ironshell", json!({"name": "math.sqrt", "args": [16]}));
    assert_eq!(res["result"]["isError"], false, "{res}");
    let v: Value = serde_json::from_str(&text_of(&res)).expect("JSON text");
    assert_eq!(v.as_f64(), Some(4.0), "{res}");
    assert_eq!(res["result"]["structuredContent"]["result"], v, "{res}");
}

#[test]
fn a_module_qualified_tool_name_is_a_tool() {
    let res = call("math.sqrt", json!({"args": [16]}));
    assert!(res.get("error").is_none(), "{res}");
    assert_eq!(res["result"]["isError"], false, "{res}");
}

#[test]
fn an_error_from_a_module_member_names_what_the_client_asked_for() {
    let res = call(
        "ironshell",
        json!({"name": "file.read", "args": ["ae-round42a-no-such-file.txt"]}),
    );
    assert_eq!(res["result"]["isError"], true, "{res}");
    let err: Value = serde_json::from_str(&text_of(&res)).expect("JSON error text");
    assert_eq!(err["error"]["builtin"], "file.read", "{err}");
}

#[test]
fn ontology_describe_of_a_member_says_which_builtin_it_runs() {
    // `implemented_by` names the builtin the member runs: file.read runs cat.
    let v = iron_shell::agent_api::ontology_describe_json("file.read");
    let by = v["implemented_by"].as_str().unwrap_or_default();
    assert!(by.ends_with("cat") || by.contains(" cat"), "{v}");
}

// 2, 3, 4
#[test]
fn a_batch_or_a_non_object_is_an_invalid_request() {
    for bad in [
        json!([]),
        json!([1, 2]),
        json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}]),
    ] {
        let res = rpc(bad).expect("never silent");
        assert_eq!(res["error"]["code"], -32600, "{res}");
        assert_eq!(res["id"], Value::Null, "{res}");
        assert!(
            res["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("batch requests are not supported in protocol 2025-06-18"),
            "{res}"
        );
    }
    for bad in [json!(42), json!("x"), json!(null), json!(true)] {
        let res = rpc(bad).expect("never silent");
        assert_eq!(res["error"]["code"], -32600, "{res}");
        assert_eq!(res["error"]["message"], "Invalid Request", "{res}");
    }
}

#[test]
fn a_null_id_is_an_invalid_request_not_a_notification() {
    let res = rpc(json!({"jsonrpc": "2.0", "id": null, "method": "ping"})).expect("a reply");
    assert_eq!(res["error"]["code"], -32600, "{res}");
    assert_eq!(res["id"], Value::Null, "{res}");
}

#[test]
fn a_request_needs_jsonrpc_2_0_and_structured_params() {
    let res = rpc(json!({"id": 5, "method": "ping"})).expect("a reply");
    assert_eq!(res["error"]["code"], -32600, "{res}");
    assert_eq!(res["id"], 5, "{res}");
    let res = rpc(json!({"jsonrpc": "1.0", "id": 6, "method": "ping"})).expect("a reply");
    assert_eq!(res["error"]["code"], -32600, "{res}");

    for params in [json!("x"), json!(3), json!(null), json!(true)] {
        let res = rpc(json!({"jsonrpc": "2.0", "id": 7, "method": "ping", "params": params}))
            .expect("a reply");
        assert_eq!(res["error"]["code"], -32602, "{res}");
        assert_eq!(res["id"], 7, "{res}");
    }
    // Structured params are fine, and a valid notification stays silent.
    assert!(rpc(json!({"jsonrpc": "2.0", "id": 8, "method": "ping", "params": []})).is_some());
    assert!(rpc(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
}

#[test]
fn the_binary_answers_every_line_including_a_batch() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["mcp", "stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn ish mcp stdio");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for line in [
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#,
            "42",
            r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#,
        ] {
            writeln!(stdin, "{line}").unwrap();
        }
    }
    let out = child.wait_with_output().expect("wait");
    let replies: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    assert_eq!(replies.len(), 4, "{replies:?}");
    assert_eq!(replies[0]["error"]["code"], -32600);
    assert_eq!(replies[1]["error"]["code"], -32600);
    assert_eq!(replies[2]["error"]["code"], -32600);
    assert_eq!(replies[3]["id"], 9);
}

// 5
#[test]
fn every_result_text_is_json_except_a_string_and_carries_structured_content() {
    // A Record is JSON, not `{bytes: 2, path: ...}`.
    let res = call("ontology_manifest", json!({}));
    assert_eq!(res["result"]["isError"], false, "{res}");
    let v: Value = serde_json::from_str(&text_of(&res)).expect("JSON text");
    assert_eq!(res["result"]["structuredContent"], v, "{res}");

    // A String is its own text, and is wrapped in structuredContent.
    let res = call("upper", json!({"args": ["hi"]}));
    assert_eq!(text_of(&res), "HI", "{res}");
    assert_eq!(res["result"]["structuredContent"], json!({"result": "HI"}));

    // An Array is a JSON array, wrapped.
    let res = call("arr.range", json!({"args": [3]}));
    assert_eq!(res["result"]["isError"], false, "{res}");
    let v: Value = serde_json::from_str(&text_of(&res)).expect("JSON text");
    assert_eq!(v, json!([0, 1, 2]), "{res}");
    assert_eq!(
        res["result"]["structuredContent"],
        json!({"result": [0, 1, 2]})
    );
}
