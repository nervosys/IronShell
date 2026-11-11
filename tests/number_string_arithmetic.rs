//! A number and a String do not combine under arithmetic.
//!
//! `"a" + 1` used to be `"a1"`, and `"a" - 1` was an error with no hint, so an
//! agent that wrote `total + 1` where `total` was text got text back without a
//! failure. Every mixed pair is now `E_BAD_ARG` naming the operator and the two
//! types, with a hint that names the conversions that exist. String + String,
//! Array + Array (concatenation), string repetition with `*`, and concatenation with a
//! non-number (`"x" + true`) are unchanged.

use iron_shell::env::Env;
use iron_shell::safety::{ErrorCode, SafetyError};
use iron_shell::value::Value;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn run(src: &str) -> anyhow::Result<Value> {
    let mut env = Env::new();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn refusal(src: &str) -> SafetyError {
    let err = run(src).expect_err("a number and a String must not combine");
    err.downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

#[test]
fn a_string_plus_a_number_is_refused_with_the_operator_and_types() {
    let _g = lock();
    let se = refusal("\"a\" + 1");
    assert_eq!(se.code, ErrorCode::BadArg);
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert_eq!(se.message, "operator Add: cannot combine String and Int");
    assert!(se.hint.contains("interpolation"), "hint: {}", se.hint);
    assert!(se.hint.contains("str("), "hint: {}", se.hint);
}

#[test]
fn a_number_plus_a_string_is_refused_in_the_other_order() {
    let _g = lock();
    let se = refusal("1 + \"a\"");
    assert_eq!(se.message, "operator Add: cannot combine Int and String");
}

#[test]
fn a_float_and_a_string_are_refused_too() {
    let _g = lock();
    assert_eq!(
        refusal("2.5 + \"a\"").message,
        "operator Add: cannot combine Float and String"
    );
    assert_eq!(
        refusal("\"a\" + 2.5").message,
        "operator Add: cannot combine String and Float"
    );
}

#[test]
fn the_other_arithmetic_operators_refuse_a_mix_too() {
    let _g = lock();
    assert_eq!(refusal("\"a\" - 1").code, ErrorCode::BadArg);
    assert_eq!(refusal("1 - \"a\"").code, ErrorCode::BadArg);
    assert_eq!(refusal("\"a\" / 2").code, ErrorCode::BadArg);
    assert_eq!(refusal("2 / \"a\"").code, ErrorCode::BadArg);
    assert_eq!(refusal("\"a\" % 2").code, ErrorCode::BadArg);
    assert_eq!(refusal("2 % \"a\"").code, ErrorCode::BadArg);
    assert_eq!(refusal("\"a\" * 1.5").code, ErrorCode::BadArg);
    assert_eq!(
        refusal("\"a\" - 1").message,
        "operator Sub: cannot combine String and Int"
    );
}

#[test]
fn a_caught_mix_carries_its_code_and_hint() {
    let _g = lock();
    assert_eq!(
        run("try { \"a\" + 1 } catch e { e.code }").unwrap(),
        Value::Str("E_BAD_ARG".to_string())
    );
}

#[test]
fn interpolation_and_str_are_the_way_to_combine_them() {
    let _g = lock();
    assert_eq!(
        run("n = 1\n\"a${n}\"").unwrap(),
        Value::Str("a1".to_string())
    );
    assert_eq!(run("str(1) + \"a\"").unwrap(), Value::Str("1a".to_string()));
    assert_eq!(
        run("to_string(2.5) + \"x\"").unwrap(),
        Value::Str("2.5x".to_string())
    );
}

#[test]
fn what_is_not_a_number_mix_is_unchanged() {
    let _g = lock();
    assert_eq!(run("\"a\" + \"b\"").unwrap(), Value::Str("ab".to_string()));
    assert_eq!(run("\"=\" * 3").unwrap(), Value::Str("===".to_string()));
    assert_eq!(run("3 * \"=\"").unwrap(), Value::Str("===".to_string()));
    assert_eq!(
        run("\"x\" + true").unwrap(),
        Value::Str("xtrue".to_string())
    );
    // Array + Array concatenates into a new Array.
    assert_eq!(
        run("[1] + [2]").unwrap(),
        Value::Array(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(run("1 + 2").unwrap(), Value::Int(3));
}
