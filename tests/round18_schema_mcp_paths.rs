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
fn item1_an_optional_subject_is_advertised_first_and_not_required() {
    let schema = claude_tool_schema("ironshell_upper");
    assert_eq!(
        schema["properties"]["subject"]["type"], "String",
        "{schema}"
    );
    assert!(
        !required_of(&schema).contains(&"subject".to_string()),
        "upper has no required parameter and is not pipeline-only: {schema}"
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
fn item2_the_server_keeps_its_announced_protocol_version() {
    let res = McpServer::new()
        .handle_rpc(&serde_json::json!({"jsonrpc":"2.0","id":3,"method":"initialize"}))
        .expect("response");
    assert_eq!(res["result"]["protocolVersion"], "2024-11-05");
}
