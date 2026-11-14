//! Round 39c: the schema's syntax block parses, its ops list is complete, the
//! suggester never offers `None`, a missing script is E_NOT_FOUND (66), and
//! `-c` takes a value that starts with a hyphen.

use std::process::Command;

fn ontology() -> serde_json::Value {
    let text = iron_shell::agent_api::generate_schema("ontology").expect("schema");
    serde_json::from_str(&text).expect("schema is JSON")
}

/// The first `syntax` object found anywhere in `v`.
fn find_key<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match v {
        serde_json::Value::Object(m) => m
            .get(key)
            .or_else(|| m.values().find_map(|c| find_key(c, key))),
        serde_json::Value::Array(a) => a.iter().find_map(|c| find_key(c, key)),
        _ => None,
    }
}

#[test]
fn every_syntax_example_in_the_schema_parses() {
    let schema = ontology();
    let syntax = find_key(&schema, "syntax")
        .and_then(|s| s.as_object())
        .expect("syntax block");
    assert!(syntax.contains_key("if"));
    for (name, example) in syntax {
        let text = example.as_str().expect("example is text");
        // The import form carries a prose note after two spaces.
        let code = text.split("  ").next().unwrap();
        iron_shell::parser::parse_program(code)
            .unwrap_or_else(|e| panic!("syntax example `{name}`: {code:?} does not parse: {e}"));
    }
}

#[test]
fn the_ops_list_names_percent_and_not() {
    let schema = ontology();
    let ops: Vec<&str> = find_key(&schema, "ops")
        .and_then(|o| o.as_array())
        .expect("ops")
        .iter()
        .filter_map(|o| o.as_str())
        .collect();
    for op in ["%", "!", "^"] {
        assert!(ops.contains(&op), "ops lacks {op}: {ops:?}");
        let src = match op {
            "!" => "!true".to_string(),
            _ => format!("7 {op} 2"),
        };
        iron_shell::parser::parse_program(&src).expect("operator parses");
    }
}

#[test]
fn the_suggester_never_offers_none_for_a_lowercase_name() {
    assert!(!iron_shell::builtins::did_you_mean("nope").contains(&"None".to_string()));
    assert!(
        !iron_shell::builtins::nearest_names("nope", ["None", "Some"].into_iter())
            .contains(&"None".to_string())
    );
}

fn ish(args: &[&str], dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish")
}

#[test]
fn a_missing_script_is_not_found_with_status_66() {
    let dir = std::env::temp_dir().join(format!("ish-r39c-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = ish(&["--agent", "nonexistent.ish"], &dir);
    assert_eq!(out.status.code(), Some(66));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("E_NOT_FOUND"), "{err}");
    let human = ish(&["nonexistent.ish"], &dir);
    assert_eq!(human.status.code(), Some(66));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dash_c_takes_a_leading_hyphen() {
    let dir = std::env::temp_dir().join(format!("ish-r39c-h-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = ish(&["-c", "-7.5 % 2"], &dir);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("-1.5"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_same_error_code_has_the_same_retryable() {
    use iron_shell::safety::{unknown_builtin, SafetyError};
    let none = unknown_builtin("zzzzzz", vec![]);
    let some = unknown_builtin("lenn", vec!["len".to_string()]);
    let r = |e: &anyhow::Error| e.downcast_ref::<SafetyError>().unwrap().is_retryable();
    assert!(!r(&none));
    assert!(r(&some));
}
