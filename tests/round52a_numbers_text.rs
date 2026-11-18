//! Round 52a: numbers, text, dates and hints.

use iron_shell::safety::SafetyError;
use iron_shell::value::Value;

fn try_run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run(src: &str) -> Value {
    try_run(src).expect("runs")
}

fn refusal(src: &str) -> SafetyError {
    let err = try_run(src).expect_err(src);
    err.downcast_ref::<SafetyError>()
        .unwrap_or_else(|| panic!("{src}: not a SafetyError: {err}"))
        .clone()
}

fn s(text: &str) -> Value {
    Value::Str(text.to_string())
}

// A1
#[test]
fn a1_int_takes_a_base_and_to_base_writes_one() {
    assert_eq!(run(r#"int("ff", 16)"#), Value::Int(255));
    assert_eq!(run(r#"int("0x1F", 16)"#), Value::Int(31));
    assert_eq!(run(r#"int("-0b101", 2)"#), Value::Int(-5));
    assert_eq!(run(r#"int("0o17", 8)"#), Value::Int(15));
    assert_eq!(run(r#"int("Z", 36)"#), Value::Int(35));
    assert_eq!(run(r#""101" | int(2)"#), Value::Int(5));
    assert_eq!(run(r#"int("42")"#), Value::Int(42));
    assert_eq!(run(r#"to_base(255, 16)"#), s("ff"));
    assert_eq!(run(r#"to_base(-255, 2)"#), s("-11111111"));
    assert_eq!(run(r#"to_base(0, 7)"#), s("0"));
    assert_eq!(
        run(r#"to_base(-9223372036854775807 - 1, 16)"#),
        s("-8000000000000000")
    );
    for src in [
        r#"int("1_0", 10)"#,
        r#"int("12", 2)"#,
        r#"int("0x", 16)"#,
        r#"int("0x-5", 16)"#,
        r#"int("ff", 37)"#,
        r#"int("ff", 1)"#,
        r#"to_base(1, 1)"#,
    ] {
        assert!(try_run(src).is_err(), "{src}");
    }
    let se = refusal(r#"int("0x1F")"#);
    assert!(se.hint.contains(r#"int("0x1F", 16)"#), "{}", se.hint);
}
