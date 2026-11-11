//! Round 18: tool schemas advertise the subject, MCP arguments map by name and
//! unknown tools are JSON-RPC errors, and a missing file inside the workspace
//! is not reported as outside it.
//!
//! Item 1 (this section): a builtin with a declared subject advertises it as a
//! first parameter named `subject` in every generated schema and in
//! `ontology_describe`, so an agent following the schema can pass the string.

use iron_shell::agent_api::{generate_schema, ontology_describe_json};
use iron_shell::mcp::McpServer;
use iron_shell::signature::{signature_of, SIGNATURES};
use serde_json::Value as JsonValue;

/// The input schema the Claude format gives the tool `name`.
fn claude_tool_schema(name: &str) -> JsonValue {
    let text = generate_schema("claude").expect("claude schema");
    let doc: JsonValue = serde_json::from_str(&text).expect("json");
    let tools = doc["tools"].as_array().expect("tools");
    let tool = tools
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("no tool {name}"));
    tool["input_schema"].clone()
}

/// The OpenAI function parameters the tool `name` is given.
fn openai_tool_parameters(name: &str) -> JsonValue {
    let text = generate_schema("openai").expect("openai schema");
    let doc: JsonValue = serde_json::from_str(&text).expect("json");
    let tools = doc["tools"].as_array().expect("tools");
    let tool = tools
        .iter()
        .find(|t| t["function"]["name"] == name)
        .unwrap_or_else(|| panic!("no tool {name}"));
    tool["function"]["parameters"].clone()
}

fn required_of(schema: &JsonValue) -> Vec<String> {
    schema["required"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn item1_a_subject_with_no_parameters_is_advertised_first_and_required() {
    // `upper` has no parameter to take its string in, so a direct call needs
    // the subject as its argument (round 20).
    let schema = claude_tool_schema("ironshell_upper");
    assert_eq!(
        schema["properties"]["subject"]["type"], "String",
        "{schema}"
    );
    assert!(
        required_of(&schema).contains(&"subject".to_string()),
        "upper has no parameter, so its subject is the argument: {schema}"
    );
}

#[test]
fn item1_a_pipeline_only_subject_is_required() {
    // `columns` is subject_required: its direct form is refused.
    let schema = claude_tool_schema("ironshell_columns");
    assert!(
        required_of(&schema).contains(&"subject".to_string()),
        "{schema}"
    );
}

#[test]
fn item1_a_subject_is_required_when_a_parameter_is_required() {
    // `pow(3)` reads its argument as the subject, so the exponent needs one.
    let schema = openai_tool_parameters("ironshell_pow");
    assert!(schema["properties"].get("subject").is_some(), "{schema}");
    assert!(
        required_of(&schema).contains(&"subject".to_string()),
        "{schema}"
    );
    assert!(
        required_of(&schema).contains(&"exponent".to_string()),
        "{schema}"
    );

    let date = claude_tool_schema("ironshell_date_add");
    let props = date["properties"].as_object().expect("properties");
    for key in ["subject", "amount", "unit"] {
        assert!(props.contains_key(key), "date_add lacks {key}: {date}");
    }
    let replace = openai_tool_parameters("ironshell_regex_replace");
    for key in ["subject", "pattern", "replacement"] {
        assert!(
            replace["properties"].get(key).is_some(),
            "regex_replace lacks {key}: {replace}"
        );
    }
}

#[test]
fn item1_ontology_describe_lists_the_subject_and_marks_it_piped() {
    let def = ontology_describe_json("upper");
    let params = def["parameters"].as_array().expect("parameters");
    assert_eq!(params.len(), 1, "{def}");
    assert_eq!(params[0]["name"], "subject", "{def}");
    assert_eq!(params[0]["type"], "String", "{def}");
    assert_eq!(params[0]["required"], false, "{def}");
    assert_eq!(params[0]["subject"], true, "{def}");
    assert_eq!(def["signature"], "String | upper() -> String", "{def}");

    // A parameter of a subject-taking builtin is not marked.
    let pow = ontology_describe_json("pow");
    let params = pow["parameters"].as_array().expect("parameters");
    assert_eq!(params[0]["name"], "subject", "{pow}");
    assert_eq!(params[1]["name"], "exponent", "{pow}");
    assert!(params[1].get("subject").is_none(), "{pow}");
}

#[test]
fn item1_every_declared_subject_is_the_first_advertised_parameter() {
    for sig in SIGNATURES {
        if sig.name.contains('-') || sig.subject.is_none() {
            continue;
        }
        if sig.params.iter().any(|p| p.name == "subject") {
            continue;
        }
        let def = ontology_describe_json(sig.name);
        let params = def["parameters"].as_array().expect("parameters");
        assert_eq!(
            params.first().and_then(|p| p["name"].as_str()),
            Some("subject"),
            "{} advertises no subject first: {def}",
            sig.name
        );
        assert_eq!(params[0]["subject"], true, "{}: {def}", sig.name);
        assert!(signature_of(sig.name).is_some());
    }
}

// ── Item 2: MCP stdio arguments and unknown tools ───────────────────────────

/// One `tools/call` through the stdio dispatcher, as a JSON-RPC response.
fn mcp_call(name: &str, arguments: JsonValue) -> JsonValue {
    McpServer::new()
        .handle_rpc(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))
        .expect("tools/call has a response")
}

/// The text of a `tools/call` result, which must be a result and not an error.
fn mcp_text(res: &JsonValue) -> String {
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
fn item2_an_empty_arguments_object_is_no_arguments() {
    let res = mcp_call("ontology_manifest", serde_json::json!({}));
    assert_eq!(res["result"]["isError"], false, "{res}");
    assert!(!mcp_text(&res).contains("expected at most"), "{res}");
}

#[test]
fn item2_ontology_describe_takes_its_name_by_name() {
    let res = mcp_call("ontology_describe", serde_json::json!({ "name": "upper" }));
    assert_eq!(res["result"]["isError"], false, "{res}");
    assert!(mcp_text(&res).contains("upper()"), "{res}");

    // The positional form the schema also advertises still works.
    let res = mcp_call(
        "ontology_describe",
        serde_json::json!({ "args": ["upper"] }),
    );
    assert_eq!(res["result"]["isError"], false, "{res}");
}

#[test]
fn item2_ontology_describe_with_no_name_is_a_missing_argument_not_a_record() {
    let res = mcp_call("ontology_describe", serde_json::json!({}));
    assert_eq!(res["result"]["isError"], true, "{res}");
    assert!(!mcp_text(&res).contains("got Record"), "{res}");
}

#[test]
fn item2_a_subject_argument_is_the_piped_value() {
    let res = mcp_call("upper", serde_json::json!({ "subject": "hi" }));
    assert_eq!(res["result"]["isError"], false, "{res}");
    assert_eq!(mcp_text(&res), "HI");
}

#[test]
fn item2_named_parameters_map_in_declaration_order() {
    let res = mcp_call(
        "date_add",
        serde_json::json!({ "subject": "2024-02-28", "amount": 1, "unit": "days" }),
    );
    assert_eq!(res["result"]["isError"], false, "{res}");
    assert_eq!(mcp_text(&res), "2024-02-29");
}

#[test]
fn item2_the_meta_tool_takes_named_arguments_too() {
    let res = mcp_call(
        "ironshell",
        serde_json::json!({ "name": "upper", "subject": "hi" }),
    );
    assert_eq!(mcp_text(&res), "HI", "{res}");
}

#[test]
fn item2_an_unknown_argument_name_is_refused() {
    let res = mcp_call("upper", serde_json::json!({ "bogus": 1 }));
    assert_eq!(res["result"]["isError"], true, "{res}");
    assert!(mcp_text(&res).contains("unknown argument 'bogus'"), "{res}");
}

#[test]
fn item2_an_unknown_tool_is_a_protocol_error_with_suggestions() {
    let res = mcp_call("bogus_tool", serde_json::json!({}));
    assert!(res.get("result").is_none(), "{res}");
    assert_eq!(res["error"]["code"], -32602, "{res}");
    assert_eq!(res["error"]["message"], "unknown tool: bogus_tool", "{res}");

    let typo = mcp_call("uppr", serde_json::json!({}));
    assert_eq!(typo["error"]["code"], -32602, "{typo}");
    let suggested = typo["error"]["data"]["did_you_mean"]
        .as_array()
        .expect("did_you_mean in error.data");
    assert!(suggested.iter().any(|v| v == "upper"), "{typo}");
}

#[test]
fn item2_a_missing_tool_name_is_invalid_params() {
    let res = McpServer::new()
        .handle_rpc(&serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "arguments": {} }
        }))
        .expect("response");
    assert_eq!(res["error"]["code"], -32602, "{res}");
    assert_eq!(res["error"]["message"], "missing tool name", "{res}");
}

#[test]
fn item2_the_server_answers_its_newest_version_when_none_is_requested() {
    // No protocolVersion in the request: the newest supported version (round 20
    // negotiates; the request's own version is covered in round20_schema_api).
    let res = McpServer::new()
        .handle_rpc(&serde_json::json!({"jsonrpc":"2.0","id":3,"method":"initialize"}))
        .expect("response");
    assert_eq!(res["result"]["protocolVersion"], "2025-06-18");
}

// ── Item 3: a missing path is judged by where it would land ─────────────────

/// `ish --agent --workspace . -c <src>` run from `ws`. Returns (exit, output).
fn agent_workspace_run(ws: &std::path::Path, src: &str) -> (i32, String) {
    let session = ws.join("session");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--agent", "--workspace", ".", "-c", src])
        .current_dir(ws)
        .env("IRONSHELL_SESSION_DIR", &session)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_ALLOW_SH", "")
        .output()
        .expect("run ish");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

fn fresh_workspace(tag: &str) -> std::path::PathBuf {
    let ws = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("r18_ws_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).expect("workspace dir");
    ws
}

#[test]
fn item3_a_missing_file_inside_the_workspace_is_not_found() {
    let ws = fresh_workspace("missing");
    let (_, text) = agent_workspace_run(&ws, r#"read_text("missing.txt")"#);
    assert!(text.contains("E_NOT_FOUND"), "{text}");
    assert!(!text.contains("E_OUTSIDE_WORKSPACE"), "{text}");
}

#[test]
fn item3_a_missing_path_through_a_missing_directory_inside_is_not_found() {
    let ws = fresh_workspace("sub_dotdot");
    let (_, text) = agent_workspace_run(&ws, r#"read_text("sub/../missing.txt")"#);
    assert!(text.contains("E_NOT_FOUND"), "{text}");
    assert!(!text.contains("E_OUTSIDE_WORKSPACE"), "{text}");
}

#[test]
fn item3_a_missing_file_one_level_up_is_outside() {
    let ws = fresh_workspace("up_one");
    let (code, text) = agent_workspace_run(&ws, r#"read_text("../missing.txt")"#);
    assert!(text.contains("E_OUTSIDE_WORKSPACE"), "{text}");
    assert!(!text.contains("E_NOT_FOUND"), "{text}");
    assert_ne!(code, 0, "{text}");
}

#[test]
fn item3_a_missing_path_that_climbs_out_is_outside() {
    let ws = fresh_workspace("climb");
    let (_, text) = agent_workspace_run(&ws, r#"read_text("nosuchdir/../../x")"#);
    assert!(text.contains("E_OUTSIDE_WORKSPACE"), "{text}");
    assert!(!text.contains("E_NOT_FOUND"), "{text}");
}

#[test]
fn item3_the_outside_refusal_names_the_builtin() {
    let ws = fresh_workspace("names");
    let (_, text) = agent_workspace_run(&ws, r#"read_text("../missing.txt")"#);
    assert!(text.contains("read_text: `"), "{text}");
    assert!(
        !text.contains("\"message\":\": `"),
        "a refusal with an empty name: {text}"
    );
}

#[cfg(unix)]
#[test]
fn item3_a_dangling_link_to_a_missing_file_outside_is_outside() {
    let ws = fresh_workspace("dangling_out");
    let outside = ws
        .parent()
        .unwrap()
        .join(format!("r18_out_{}", std::process::id()));
    std::os::unix::fs::symlink(outside.join("target.txt"), ws.join("out")).unwrap();
    let (_, text) = agent_workspace_run(&ws, r#"read_text("out")"#);
    assert!(text.contains("E_OUTSIDE_WORKSPACE"), "{text}");
}

#[cfg(unix)]
#[test]
fn item3_a_dangling_link_to_a_missing_file_inside_is_not_found() {
    let ws = fresh_workspace("dangling_in");
    std::os::unix::fs::symlink(ws.join("missing.txt"), ws.join("in")).unwrap();
    let (_, text) = agent_workspace_run(&ws, r#"read_text("in")"#);
    assert!(text.contains("E_NOT_FOUND"), "{text}");
}

#[test]
fn item3_a_plain_missing_file_stays_not_found_outside_agent_mode() {
    let ws = fresh_workspace("plain");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", r#"read_text("missing.txt")"#])
        .current_dir(&ws)
        .env("IRONSHELL_SESSION_DIR", ws.join("session"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run ish");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("E_NOT_FOUND"), "{text}");
}

// ── Item 4: did-you-mean for names agents guess ─────────────────────────────

#[test]
fn item4_guessed_read_names_point_at_read_text_first() {
    use iron_shell::builtins::did_you_mean;
    for guess in ["file_read", "read_file", "readfile"] {
        let dym = did_you_mean(guess);
        assert_eq!(
            dym.first().map(String::as_str),
            Some("read_text"),
            "{guess}: {dym:?}"
        );
        assert!(dym.iter().any(|s| s == "file.read"), "{guess}: {dym:?}");
    }
}

#[test]
fn item4_exists_file_points_at_exists_and_repeat_at_pad_left() {
    use iron_shell::builtins::did_you_mean;
    assert_eq!(
        did_you_mean("exists_file").first().map(String::as_str),
        Some("exists")
    );
    assert_eq!(
        did_you_mean("repeat").first().map(String::as_str),
        Some("pad_left")
    );
}

#[test]
fn item4_every_suggested_name_is_real() {
    use iron_shell::builtins::is_served;
    for name in ["read_text", "exists", "pad_left"] {
        assert!(is_served(name), "{name} is suggested but not served");
    }
    assert!(
        iron_shell::modules::builtin_for_member("file", "read").is_some(),
        "file.read is suggested but is not a module member"
    );
}

#[test]
fn item4_the_cli_names_the_real_builtin_for_file_read() {
    let ws = fresh_workspace("guess");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", r#"file_read("x")"#])
        .current_dir(&ws)
        .env("IRONSHELL_SESSION_DIR", ws.join("session"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run ish");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("E_UNKNOWN_BUILTIN"), "{text}");
    assert!(
        text.contains("did you mean: read_text, file.read"),
        "{text}"
    );
}
