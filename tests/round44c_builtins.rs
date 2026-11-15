//! Round 44c: piped forms, declarations that match the run, and error names.

use iron_shell::value::Value;

fn try_run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run(src: &str) -> Value {
    try_run(src).expect("runs")
}

#[test]
fn pow_takes_its_base_from_the_pipe() {
    assert_eq!(run("2 | pow(3)"), Value::Int(8));
    assert_eq!(run("2 | math.pow(3)"), Value::Int(8));
    assert_eq!(run("pow(2, 3)"), Value::Int(8));
}

#[test]
fn the_uniq_help_does_not_promise_a_flag_the_call_refuses() {
    assert!(try_run(r#"uniq([1, 1, 2], "-c")"#).is_err());
    let sig = iron_shell::signature::signature_of("uniq").expect("declared");
    assert!(!sig.doc.contains("accepted and ignored"), "{}", sig.doc);
}

#[test]
fn hash_refuses_two_algorithm_names_and_takes_the_algorithm_first() {
    assert!(try_run(r#"hash("md5", "sha256")"#).is_err());
    assert_eq!(
        run(r#"crypto_hash("md5", "hello")"#),
        Value::Str("5d41402abc4b2a76b9719d911017c592".into())
    );
}

#[test]
fn sort_by_declares_its_key_required_and_a_missing_key_is_refused() {
    let sig = iron_shell::signature::signature_of("sort_by").expect("declared");
    assert!(sig.params[0].required, "key must be declared required");
    let err = try_run("[3, 1, 2] | sort_by").expect_err("no key");
    let text = err.to_string();
    assert!(
        text.contains("field name") && text.contains("lambda"),
        "{text}"
    );
}

#[test]
fn contains_tests_array_membership_and_record_keys() {
    assert_eq!(run("contains([1, 2], 1)"), Value::Bool(true));
    assert_eq!(run("contains([1, 2], 1.0)"), Value::Bool(true));
    assert_eq!(run("[1, 2] | contains(3)"), Value::Bool(false));
    assert_eq!(run(r#"contains({a: 1}, "a")"#), Value::Bool(true));
    assert_eq!(run(r#"{a: 1} | contains("b")"#), Value::Bool(false));
    assert_eq!(run(r#"contains("hello", "ell")"#), Value::Bool(true));
    assert!(try_run("contains({a: 1}, 1)").is_err());
}

/// The error's message. A coded error renders as one line of JSON, so the
/// message is read out of it; any other error is its own text.
fn error_text(src: &str) -> String {
    let text = try_run(src).expect_err("fails").to_string();
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|j| j["error"]["message"].as_str().map(str::to_string))
        .unwrap_or(text)
}

#[test]
fn errors_name_the_builtin_as_called() {
    // Real names, not the hyphenated spellings that are not builtins.
    assert!(
        error_text("from_json()").starts_with("from_json:"),
        "from_json"
    );
    assert!(error_text("from_yaml()").contains("from_yaml"), "from_yaml");
    assert!(!error_text("from_yaml()").contains("from-yaml"));
    assert!(!error_text("to_yaml()").contains("to-yaml"));
    assert!(!error_text("from_csv(1)").contains("from-csv"));
    // An alias spelling names itself, not the internal crypto_ name.
    let text = error_text(r#"hash("md5", "sha256")"#);
    assert!(text.starts_with("hash:"), "{text}");
}

#[test]
fn the_round_help_states_the_digits_range() {
    let sig = iron_shell::signature::signature_of("round").expect("declared");
    assert!(sig.doc.contains("0 to 17"), "{}", sig.doc);
    assert!(
        sig.params[0].doc.contains("0 to 17"),
        "{}",
        sig.params[0].doc
    );
    assert!(try_run("round(1.5, 18)").is_err());
    assert!(try_run("round(1.5, 17)").is_ok());
}
