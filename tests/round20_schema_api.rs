//! Round 20: tool schemas list `subject` first and their parameters in
//! declaration order, a direct call's required list names what the builtin
//! cannot run without, `ontology_describe` of a builtin carries its
//! `json_schema`, a request that repeats a top-level key is refused, and MCP
//! `initialize` negotiates the protocol version.

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
