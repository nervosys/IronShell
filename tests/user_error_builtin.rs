//! `error(message[, extra])` raises an error the script chose.
//!
//! It is `E_USER` with the message as given. A Record `extra` adds its fields to
//! the caught error record, but never replaces a standard field. It is catchable
//! with try/catch, and uncaught it is a failure with exit status 1.

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
    let err = run(src).expect_err("error() must raise");
    err.downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

#[test]
fn error_is_catchable_with_its_message_as_given() {
    let _g = lock();
    assert_eq!(
        run("try { error(\"boom\") } catch e { e.message }").unwrap(),
        Value::Str("boom".to_string())
    );
}

#[test]
fn error_is_e_user() {
    let _g = lock();
    assert_eq!(
        run("try { error(\"bad\") } catch e { e.code }").unwrap(),
        Value::Str("E_USER".to_string())
    );
    assert_eq!(
        run("try { error(\"bad\") } catch e { e.error.code }").unwrap(),
        Value::Str("E_USER".to_string())
    );
}

#[test]
fn an_extra_record_adds_fields_to_the_caught_error() {
    let _g = lock();
    assert_eq!(
        run("try { error(\"bad\", {field: \"age\"}) } catch e { e.field }").unwrap(),
        Value::Str("age".to_string())
    );
}

#[test]
fn an_extra_field_named_like_a_standard_one_is_refused() {
    let _g = lock();
    for src in [
        "error(\"bad\", {code: \"MINE\", message: \"x\"})",
        "error(\"bad\", {code: \"MINE\"})",
    ] {
        let msg = run(src).unwrap_err().to_string();
        assert!(msg.contains("E_BAD_ARG"), "{src}: {msg}");
        assert!(
            msg.contains("rename the field, e.g. {user_code: 7}"),
            "{src}: {msg}"
        );
    }
}

#[test]
fn the_message_can_come_from_the_pipeline() {
    let _g = lock();
    assert_eq!(
        run("try { \"boom\" | error() } catch e { e.message }").unwrap(),
        Value::Str("boom".to_string())
    );
}

#[test]
fn uncaught_it_is_a_failure_with_exit_status_one() {
    let _g = lock();
    let se = refusal("error(\"boom\")");
    assert_eq!(se.code, ErrorCode::User);
    assert_eq!(se.message, "boom");
    assert_eq!(se.code.exit_code(), 1);
    assert!(!se.is_retryable());
}

#[test]
fn a_non_string_message_is_refused() {
    let _g = lock();
    assert_eq!(refusal("error(5)").code, ErrorCode::BadArg);
}

#[test]
fn a_non_record_extra_is_refused() {
    let _g = lock();
    assert_eq!(refusal("error(\"x\", 5)").code, ErrorCode::BadArg);
}
