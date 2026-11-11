//! Correctness defects found by an agent tester (round 15).
//!
//! `if` desugars to a two-arm match on `true`. A condition that is not a Bool
//! (a Record from `exists()`, an Int, a String) used to take the else branch
//! silently; it is now an E_BAD_ARG. Null stays false, and `where` keeps its
//! truthiness.

use iron_shell::value::Value;

fn run(src: &str) -> Result<Value, String> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).map_err(|e| e.to_string())
}

// ── Item 1: a non-Bool `if` condition is an error, not a silent else ──

#[test]
fn an_if_on_a_record_is_an_error_not_a_silent_else() {
    let err = run(r#"r = {exists: true}; if r { "yes" } else { "no" }"#)
        .expect_err("a Record condition must be refused");
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(err.contains("Bool"), "{err}");
    assert!(
        err.contains("exists(p).exists") || err.contains("field"),
        "the hint says to test a field: {err}"
    );
}

#[test]
fn an_if_on_an_int_is_an_error() {
    let err = run(r#"n = 0; if n { "yes" } else { "no" }"#).expect_err("an Int is refused");
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(err.contains("n != 0"), "{err}");
}

#[test]
fn an_if_on_a_string_is_an_error() {
    let err = run(r#"s = ""; if s { "yes" } else { "no" }"#).expect_err("a String is refused");
    assert!(err.contains("E_BAD_ARG"), "{err}");
}

#[test]
fn an_if_on_an_array_is_an_error() {
    let err = run(r#"a = []; if a { "yes" } else { "no" }"#).expect_err("an Array is refused");
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(err.contains("len(a) > 0"), "{err}");
}

#[test]
fn an_if_on_null_stays_false() {
    assert_eq!(
        run(r#"if null { "yes" } else { "no" }"#),
        Ok(Value::Str("no".into()))
    );
    assert_eq!(run("if null { 1 }"), Ok(Value::Null));
}

#[test]
fn an_if_on_a_bool_and_a_comparison_still_work() {
    assert_eq!(
        run(r#"if 1 < 2 { "yes" } else { "no" }"#),
        Ok(Value::Str("yes".into()))
    );
    assert_eq!(
        run(r#"if false { "yes" } else if true { "second" } else { "no" }"#),
        Ok(Value::Str("second".into()))
    );
}

#[test]
fn an_if_comparing_a_record_field_works() {
    assert_eq!(
        run(r#"r = {exists: true}; if r.exists { "yes" } else { "no" }"#),
        Ok(Value::Str("yes".into()))
    );
}

#[test]
fn where_still_takes_a_truthy_value() {
    // where/&&/||/! keep truthiness: only `if` is strict.
    assert_eq!(
        run("[0, 1, 2] | where(fn(x) => x)"),
        Ok(Value::Array(vec![Value::Int(1), Value::Int(2)]))
    );
}
