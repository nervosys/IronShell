//! Round 20: tool schemas list `subject` first and their parameters in
//! declaration order, a direct call's required list names what the builtin
//! cannot run without, `ontology_describe` of a builtin carries its
//! `json_schema`, a request that repeats a top-level key is refused, and MCP
//! `initialize` negotiates the protocol version.

use iron_shell::agent_api::{ontology_describe_json, parse_request, process_request};
use iron_shell::mcp::{negotiate_protocol_version, McpServer};
use serde_json::Value as JsonValue;

/// The OpenAI function parameters the tool `name` is given.
fn openai_tool_parameters(name: &str) -> JsonValue {
    let text = iron_shell::agent_api::generate_schema("openai").expect("openai schema");
    let doc: JsonValue = serde_json::from_str(&text).expect("json");
    let tools = doc["tools"].as_array().expect("tools");
    let tool = tools
        .iter()
        .find(|t| t["function"]["name"] == name)
        .unwrap_or_else(|| panic!("no tool {name}"));
    tool["function"]["parameters"].clone()
}

#[test]
fn item1_subject_is_the_first_property_and_parameters_keep_declaration_order() {
    // Sorted keys would give content before path; declaration order is path, content.
    let schema = openai_tool_parameters("ironshell_file_write");
    let keys: Vec<&str> = schema["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, vec!["path", "content"], "{schema}");

    // A builtin with a subject: the subject leads, then its parameter.
    let pow = openai_tool_parameters("ironshell_pow");
    let keys: Vec<&str> = pow["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, vec!["subject", "exponent"], "{pow}");
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
fn item2_a_subject_with_no_parameters_is_required_in_tool_schemas() {
    // Each runs only with its subject: a direct call has no pipeline to supply it.
    for name in [
        "ironshell_sqrt",
        "ironshell_upper",
        "ironshell_len",
        "ironshell_avg",
    ] {
        let schema = openai_tool_parameters(name);
        assert!(
            required_of(&schema).contains(&"subject".to_string()),
            "{name} lists its subject as optional: {schema}"
        );
    }
}

#[test]
fn item2_a_subject_with_only_optional_parameters_stays_optional() {
    // `wc` takes its subject as text or a file, and its flags are optional.
    let schema = openai_tool_parameters("ironshell_wc");
    assert!(
        !required_of(&schema).contains(&"subject".to_string()),
        "wc: {schema}"
    );
}

#[test]
fn item2_a_parameter_that_defaults_to_the_pipeline_is_required() {
    // `file_write`'s content is "omitted uses pipeline input", so a direct call
    // that leaves it out has nothing to write: the schema requires it.
    let schema = openai_tool_parameters("ironshell_file_write");
    let required = required_of(&schema);
    assert!(required.contains(&"path".to_string()), "{schema}");
    assert!(required.contains(&"content".to_string()), "{schema}");
}

#[test]
fn item3_ontology_describe_carries_the_json_schema_the_describe_action_gives() {
    for name in ["pow", "upper", "split", "file_write"] {
        let described = ontology_describe_json(name);
        let text = format!(r#"{{"action":"describe","builtin":"{name}"}}"#);
        let request = parse_request(&text)
            .unwrap_or_else(|_| panic!("describe request for {name} does not parse"));
        let response = process_request(&request);
        let result = response
            .result
            .expect("describe action answers a definition");

        assert!(!described["json_schema"].is_null(), "{name}: {described}");
        assert_eq!(described["json_schema"], result["json_schema"], "{name}");
    }
}

#[test]
fn item4_a_request_that_repeats_a_top_level_key_is_refused() {
    // Last-wins would run `eval`; the request is refused and names the key.
    let text = r#"{"action":"call","builtin":"upper","args":[],"action":"eval"}"#;
    let err = parse_request(text).expect_err("a repeated key is refused");
    assert_eq!(err.code.as_deref(), Some("E_PARSE"), "{err:?}");
    let message = err.error.unwrap_or_default();
    assert!(message.contains("duplicate key \"action\""), "{message}");

    let response = iron_shell::agent_api::process_json_response(text);
    assert!(!response.success, "{response:?}");
    assert_eq!(response.code.as_deref(), Some("E_PARSE"), "{response:?}");
}

#[test]
fn item4_a_repeated_key_nested_below_the_top_level_is_not_refused() {
    // The check is for the request object only.
    let text = r#"{"action":"eval","code":"1 + 1","meta":{"x":1,"x":2}}"#;
    assert!(
        parse_request(text).is_ok(),
        "nested repeats are not refused"
    );
}

/// The protocol version the server answers an `initialize` with `params`.
fn answered_version(params: Option<serde_json::Value>) -> String {
    let mut req = serde_json::json!({"jsonrpc": "2.0", "id": 7, "method": "initialize"});
    if let Some(p) = params {
        req["params"] = p;
    }
    let res = McpServer::new().handle_rpc(&req).expect("response");
    res["result"]["protocolVersion"]
        .as_str()
        .expect("protocolVersion")
        .to_string()
}

#[test]
fn item5_a_supported_requested_version_is_echoed() {
    for version in ["2025-06-18", "2025-03-26", "2024-11-05"] {
        let got = answered_version(Some(
            serde_json::json!({"protocolVersion": version, "capabilities": {}}),
        ));
        assert_eq!(got, version);
    }
}

#[test]
fn item5_an_unknown_or_missing_requested_version_gets_the_newest_supported() {
    let unknown = answered_version(Some(serde_json::json!({"protocolVersion": "1999-01-01"})));
    assert_eq!(unknown, "2025-06-18");
    let absent = answered_version(Some(serde_json::json!({"capabilities": {}})));
    assert_eq!(absent, "2025-06-18");
    assert_eq!(answered_version(None), "2025-06-18");
    assert_eq!(negotiate_protocol_version(Some("2024-11-05")), "2024-11-05");
    assert_eq!(negotiate_protocol_version(Some("bogus")), "2025-06-18");
}
