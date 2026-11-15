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
