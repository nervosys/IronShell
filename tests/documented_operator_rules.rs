//! The operator rules AGENTS.md states, and the `error` parameter doc.

use iron_shell::value::Value;

fn try_run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

#[test]
fn slash_on_two_ints_is_true_division() {
    assert_eq!(try_run("6 / 2").unwrap(), Value::Float(3.0));
    assert_eq!(try_run("math.floor(7 / 2)").unwrap(), Value::Int(3));
}

#[test]
fn not_follows_truthiness() {
    for (src, want) in [
        ("!0", true),
        ("!\"\"", true),
        ("!null", true),
        ("![]", true),
        ("!1", false),
        ("!\"a\"", false),
    ] {
        assert_eq!(try_run(src).unwrap(), Value::Bool(want), "{src}");
    }
}

#[test]
fn if_and_logic_operators_take_only_bool() {
    assert!(try_run("if 0 { 1 } else { 2 }").is_err());
    assert!(try_run("1 && 2").is_err());
    assert!(try_run("0 || true").is_err());
}

#[test]
fn error_message_param_is_documented_as_required_unless_piped() {
    let sig = iron_shell::signature::signature_of("error").expect("a declared signature");
    let doc = sig
        .params
        .iter()
        .find(|p| p.name == "message")
        .map(|p| p.doc)
        .expect("a message parameter");
    assert!(doc.contains("required unless a value is piped"), "{doc}");
}
