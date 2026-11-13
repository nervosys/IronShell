//! Round 31: the Agent API's provider schemas are JSON Schema, `sql` takes a
//! lone query by name, `/eval` refuses a missing `code`, a `/call` body wrapped
//! as `{"args": ...}` is unwrapped, module-qualified names resolve for
//! describe and call, and an example does not repeat its builtin's description.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use iron_shell::agent_api::{
    eval_code_from_body, generate_schema, ontology_describe_json, process_request,
    unknown_builtin_refusal, AgentRequest, AgentResponse,
};
use serde_json::{json, Value as JsonValue};

/// Serialises the tests that set `IRONSHELL_SESSION_DIR`.
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct Sandbox {
    _guard: MutexGuard<'static, ()>,
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root =
            std::env::temp_dir().join(format!("ish-round31-api-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("session")).expect("create session dir");
        std::env::set_var("IRONSHELL_SESSION_DIR", root.join("session"));
        Sandbox {
            _guard: guard,
            root,
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        std::env::remove_var("IRONSHELL_SESSION_DIR");
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn call(builtin: &str, args: JsonValue) -> AgentResponse {
    process_request(&AgentRequest::Call {
        builtin: builtin.to_string(),
        args,
    })
}

fn describe(name: &str) -> AgentResponse {
    process_request(&AgentRequest::Describe {
        builtin: name.to_string(),
    })
}

fn schema(format: &str) -> JsonValue {
    serde_json::from_str(&generate_schema(format).expect("schema")).expect("json")
}

const JSON_SCHEMA_TYPES: [&str; 7] = [
    "string", "integer", "number", "boolean", "array", "object", "null",
];

/// Every `type` in a schema (the value of a `"type"` key that is a string or a
/// list of strings), collected through the whole tree. A property that is
/// itself called `type` holds an object and is not collected.
fn collect_types(v: &JsonValue, out: &mut Vec<String>) {
    match v {
        JsonValue::Object(map) => {
            for (key, child) in map {
                match (key.as_str(), child) {
                    ("type", JsonValue::String(s)) => out.push(s.clone()),
                    ("type", JsonValue::Array(a)) => {
                        out.extend(a.iter().filter_map(|t| t.as_str().map(str::to_string)))
                    }
                    _ => collect_types(child, out),
                }
            }
        }
        JsonValue::Array(items) => items.iter().for_each(|i| collect_types(i, out)),
        _ => {}
    }
}

fn assert_json_schema_types(parameters: &JsonValue, who: &str) {
    let mut types = Vec::new();
    collect_types(parameters, &mut types);
    for t in types {
        assert!(
            JSON_SCHEMA_TYPES.contains(&t.as_str()),
            "{who}: '{t}' is not a JSON Schema type in {parameters}"
        );
    }
}

#[test]
fn openai_and_claude_schemas_use_only_json_schema_types() {
    let openai = schema("openai");
    let tools = openai["tools"].as_array().expect("tools");
    assert!(tools.len() > 1000, "{} tools", tools.len());
    for tool in tools {
        let f = &tool["function"];
        assert_json_schema_types(&f["parameters"], f["name"].as_str().unwrap_or("?"));
    }
    let claude = schema("claude");
    for tool in claude["tools"].as_array().expect("tools") {
        assert_json_schema_types(&tool["input_schema"], tool["name"].as_str().unwrap_or("?"));
    }
    // The other OpenAI-shaped families share the generator.
    for format in ["azure_openai", "groq", "mistral"] {
        let doc = schema(format);
        for tool in doc["tools"].as_array().expect("tools") {
            assert_json_schema_types(&tool["function"]["parameters"], format);
        }
    }
    let bedrock = schema("bedrock");
    for tool in bedrock["toolConfig"]["tools"].as_array().expect("tools") {
        assert_json_schema_types(&tool["toolSpec"]["inputSchema"]["json"], "bedrock");
    }
}

#[test]
fn an_any_parameter_has_no_type_and_a_lambda_is_source_text() {
    let openai = schema("openai");
    let mut any_untyped = 0;
    let mut lambda_source = 0;
    for tool in openai["tools"].as_array().expect("tools") {
        let props = &tool["function"]["parameters"]["properties"];
        for (_, p) in props.as_object().into_iter().flatten() {
            if p.get("type").is_none() {
                any_untyped += 1;
            }
            if p["description"]
                .as_str()
                .is_some_and(|d| d.contains("IronShell lambda source"))
            {
                lambda_source += 1;
                assert_eq!(p["type"], "string", "{p}");
            }
        }
    }
    assert!(any_untyped > 0, "Any parameters carry no type key");
    assert!(lambda_source > 0, "Lambda parameters are marked as source");
}

#[test]
fn a_union_type_is_a_list_of_json_schema_types() {
    // `sort` takes an array or text.
    let openai = schema("openai");
    let mut unions = 0;
    for tool in openai["tools"].as_array().expect("tools") {
        for (_, p) in tool["function"]["parameters"]["properties"]
            .as_object()
            .into_iter()
            .flatten()
        {
            if let Some(list) = p["type"].as_array() {
                unions += 1;
                assert!(list.len() >= 2, "{p}");
            }
        }
    }
    assert!(unions > 0, "Array | String becomes [\"array\", \"string\"]");
}

#[test]
fn the_ontology_format_keeps_ironshell_type_names() {
    let ontology = describe("upper").result.expect("describe upper");
    assert_eq!(ontology["parameters"][0]["type"], "String", "{ontology}");
}

#[test]
fn gemini_declarations_use_its_own_subset() {
    let gemini = schema("gemini");
    let decls = gemini["function_declarations"].as_array().expect("decls");
    let mut checked = 0;
    for d in decls {
        if let Some(params) = d.get("parameters") {
            let text = params.to_string();
            assert!(!text.contains("\"x-"), "{text}");
            let mut types = Vec::new();
            collect_types(params, &mut types);
            for t in types {
                assert!(
                    ["STRING", "INTEGER", "NUMBER", "BOOLEAN", "ARRAY", "OBJECT", "NULL"]
                        .contains(&t.as_str()),
                    "{t} in {text}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 1000);
}

#[test]
fn no_openai_description_is_over_1024_characters() {
    let openai = schema("openai");
    let mut truncated = 0;
    for tool in openai["tools"].as_array().expect("tools") {
        let d = tool["function"]["description"].as_str().expect("desc");
        assert!(d.chars().count() <= 1024, "{}", tool["function"]["name"]);
        if d.ends_with('\u{2026}') {
            truncated += 1;
        }
    }
    assert!(truncated > 0, "some description is over the limit");
    // The other families are not capped by this rule.
    let claude = schema("claude");
    // Each cut is at a word boundary: the text before the ellipsis is a
    // prefix of the full description that ends before whitespace.
    let mut compared = 0;
    for tool in openai["tools"].as_array().unwrap() {
        let cut = tool["function"]["description"].as_str().unwrap();
        if !cut.ends_with('\u{2026}') {
            continue;
        }
        let name = tool["function"]["name"].as_str().unwrap();
        let Some(full) = claude["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"].as_str() == Some(name))
            .and_then(|t| t["description"].as_str())
        else {
            continue;
        };
        let head = cut.trim_end_matches('\u{2026}');
        assert!(full.starts_with(head), "{name}: {cut}");
        assert!(
            full[head.len()..].starts_with(char::is_whitespace),
            "{name}: cut inside a word: {cut}"
        );
        compared += 1;
    }
    assert!(compared > 0, "no truncated description could be compared");
    let long = claude["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["description"].as_str().unwrap_or("").chars().count() > 1024);
    assert!(long, "claude keeps the full description");
}

#[test]
fn sql_takes_a_lone_query_by_name() {
    let _sandbox = Sandbox::new("sql");
    let r = call("sql", json!({"source_or_query": "select 1 as x"}));
    assert!(r.success, "{:?}", r.error);
    assert_eq!(r.result, Some(json!([{"x": 1}])));

    let v = call("sql_value", json!({"source_or_query": "select 1 + 1"}));
    assert!(v.success, "{:?}", v.error);
    assert_eq!(v.result, Some(json!(2)));

    // A source and a query still work.
    let both = call(
        "sql",
        json!({"source_or_query": ":memory:", "query": "select 7 as y"}),
    );
    assert!(both.success, "{:?}", both.error);

    // The schema no longer makes the subject required.
    let openai = schema("openai");
    for name in ["ironshell_sql", "ironshell_sql_value"] {
        let tool = openai["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["function"]["name"] == name)
            .expect("tool");
        let required: Vec<&str> = tool["function"]["parameters"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(JsonValue::as_str)
            .collect();
        assert!(!required.contains(&"subject"), "{name}: {required:?}");
        assert!(
            required.contains(&"source_or_query"),
            "{name}: {required:?}"
        );
    }
}

#[test]
fn eval_without_code_is_refused_naming_the_field() {
    for body in [json!({}), json!({"code": 5}), json!({"code": null})] {
        let refusal = eval_code_from_body(&body).expect_err("refused");
        assert!(!refusal.success);
        assert_eq!(refusal.code.as_deref(), Some("E_BAD_ARG"));
        assert!(
            refusal.error.as_deref().unwrap_or("").contains("'code'"),
            "{:?}",
            refusal.error
        );
    }
    assert_eq!(
        eval_code_from_body(&json!({"code": "1 + 1"})).unwrap(),
        "1 + 1"
    );
    // The empty program is still a string, and still allowed.
    assert!(eval_code_from_body(&json!({"code": ""})).is_ok());
}

#[test]
fn an_args_envelope_is_unwrapped() {
    let _sandbox = Sandbox::new("envelope");
    let plain = call("crypto_hmac", json!({"data": "message", "key": "secret"}));
    assert!(plain.success, "{:?}", plain.error);
    let wrapped = call(
        "crypto_hmac",
        json!({"args": {"data": "message", "key": "secret"}}),
    );
    assert!(wrapped.success, "{:?}", wrapped.error);
    assert_eq!(wrapped.result, plain.result);
    let positional = call("crypto_hmac", json!({"args": ["message", "secret"]}));
    assert_eq!(positional.result, plain.result);

    // Another unknown key still names the valid parameters and the body shape.
    let bad = call("crypto_hmac", json!({"data": "m", "key": "k", "hash": "x"}));
    assert!(!bad.success);
    let hint = bad.hint.unwrap_or_default();
    assert!(hint.contains("arguments object itself"), "{hint}");
}

#[test]
fn an_unknown_builtin_is_unknown_not_an_unknown_argument() {
    let _sandbox = Sandbox::new("unknown");
    let r = call("nonexistent", json!({"args": {"x": 1}}));
    assert!(!r.success);
    assert_eq!(r.code.as_deref(), Some("E_UNKNOWN_BUILTIN"), "{r:?}");
    let error = r.error.unwrap_or_default();
    assert!(!error.contains("unknown argument"), "{error}");
    assert!(unknown_builtin_refusal("nonexistent").is_some());
    assert!(unknown_builtin_refusal("upper").is_none());
    // A near miss gets suggestions.
    let near = call("uper", json!({}));
    assert_eq!(near.code.as_deref(), Some("E_UNKNOWN_BUILTIN"), "{near:?}");
    assert!(near.hint.unwrap_or_default().contains("upper"));
}

#[test]
fn module_qualified_names_resolve_for_describe_and_call() {
    let _sandbox = Sandbox::new("module");
    for name in ["file.read", "math.sqrt", "str.upper"] {
        let d = describe(name);
        assert!(d.success, "{name}: {:?}", d.error);
        assert!(unknown_builtin_refusal(name).is_none(), "{name}");
    }
    let r = call("math.sqrt", json!([16]));
    assert!(r.success, "{:?}", r.error);
    assert_eq!(r.result.as_ref().and_then(JsonValue::as_f64), Some(4.0));
    let bad = describe("nosuchmodule.read");
    assert!(!bad.success);
    assert_eq!(bad.code.as_deref(), Some("E_UNKNOWN_BUILTIN"));
}

#[test]
fn an_example_does_not_repeat_the_builtins_description() {
    for name in ["date_diff", "sql", "upper"] {
        let d = ontology_describe_json(name);
        let description = d["description"].as_str().expect("description");
        let examples = d["examples"].as_array().expect("examples");
        assert!(!examples.is_empty(), "{name}");
        for e in examples {
            assert!(e["code"].is_string(), "{e}");
            assert_ne!(e["description"].as_str(), Some(description), "{name}");
        }

        // The same through the HTTP-shaped describe request.
        let via = describe(name).result.expect("describe");
        for e in via["examples"].as_array().expect("examples") {
            assert_ne!(e["description"].as_str(), Some(description), "{name}");
        }
    }
    assert!(
        ontology_describe_json("date_diff").to_string().len() < 6000,
        "date_diff's detail is no longer ~13 KB"
    );
}
