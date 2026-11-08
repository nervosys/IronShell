//! An Int result that does not fit in 64 bits is an error, not a wrapped number.
//!
//! `+`, `-`, `*`, `%`, `**` and unary minus on Int used to wrap (or panic in a
//! debug build), and the wrapped number went on as an answer. They are now
//! `E_BAD_VALUE` ("integer overflow in operator Add"), not retryable. Results
//! that fit, and every Float operation, are unchanged.

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
    let err = run(src).expect_err("an overflowing Int operation must be refused");
    err.downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

#[test]
fn addition_past_i64_max_is_refused() {
    let _g = lock();
    let se = refusal("9223372036854775807 + 1");
    assert_eq!(se.code, ErrorCode::Overflow);
    assert_eq!(se.code.as_str(), "E_BAD_VALUE");
    assert_eq!(se.message, "integer overflow in operator Add");
}

#[test]
fn subtraction_past_i64_min_is_refused() {
    let _g = lock();
    assert_eq!(
        refusal("-9223372036854775807 - 2").message,
        "integer overflow in operator Sub"
    );
}

#[test]
fn multiplication_past_i64_max_is_refused() {
    let _g = lock();
    assert_eq!(
        refusal("4611686018427387904 * 2").message,
        "integer overflow in operator Mul"
    );
}

#[test]
fn power_past_i64_max_is_refused() {
    let _g = lock();
    assert_eq!(
        refusal("2 ^ 63").message,
        "integer overflow in operator Pow"
    );
}

#[test]
fn negating_i64_min_is_refused() {
    let _g = lock();
    let se = refusal("x = -9223372036854775807 - 1; -x");
    assert_eq!(se.message, "integer overflow in operator Neg");
}

#[test]
fn remainder_of_i64_min_by_minus_one_is_refused() {
    let _g = lock();
    assert_eq!(
        refusal("x = -9223372036854775807 - 1; x % -1").message,
        "integer overflow in operator Rem"
    );
}

#[test]
fn results_that_fit_are_unchanged() {
    let _g = lock();
    assert_eq!(
        run("9223372036854775806 + 1").unwrap(),
        Value::Int(i64::MAX)
    );
    assert_eq!(run("2 ^ 62").unwrap(), Value::Int(1 << 62));
    assert_eq!(run("7 % 3").unwrap(), Value::Int(1));
    assert_eq!(run("-5 * 4").unwrap(), Value::Int(-20));
    assert_eq!(run("0 ^ 0").unwrap(), Value::Int(1));
}

#[test]
fn floats_do_not_overflow_into_an_error() {
    let _g = lock();
    assert_eq!(run("1.5 + 2").unwrap(), Value::Float(3.5));
}

#[test]
fn a_caught_overflow_carries_its_code() {
    let _g = lock();
    assert_eq!(
        run("try { 9223372036854775807 + 1 } catch e { e.code }").unwrap(),
        Value::Str("E_BAD_VALUE".to_string())
    );
}

#[test]
fn overflow_is_a_usage_failure_that_is_not_retryable() {
    assert_eq!(ErrorCode::Overflow.as_str(), "E_BAD_VALUE");
    assert!(!ErrorCode::Overflow.retryable());
    assert_eq!(ErrorCode::Overflow.exit_code(), 64);
}
