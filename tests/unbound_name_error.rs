//! A bare name that nothing binds is an error, not null.
//!
//! It used to evaluate to Null, so `countr` (a misspelling of `counter`) ran on
//! with null and failed, or did not fail, far from its cause. It is now
//! `E_UNKNOWN_NAME`, with `did_you_mean` drawn from the bound variables and the
//! builtins. Names that are bound (variables, lambda parameters, match and catch
//! bindings, modules, builtins as values) are unaffected.

use iron_shell::safety::{ErrorCode, SafetyError};
use iron_shell::value::Value;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn run(src: &str) -> anyhow::Result<Value> {
    // The shell's own environment, with the module namespaces bound, as the
    // REPL and scripts run.
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn refusal(src: &str) -> SafetyError {
    let err = run(src).expect_err("an unbound name must be refused");
    err.downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

#[test]
fn a_bare_unbound_name_is_e_unknown_name() {
    let _g = lock();
    let se = refusal("nosuchname_zz");
    assert_eq!(se.code, ErrorCode::UnknownName);
    assert_eq!(se.code.as_str(), "E_UNKNOWN_NAME");
    assert_eq!(se.message, "unknown name `nosuchname_zz`");
}

#[test]
fn an_unbound_name_suggests_the_nearest_bound_variable() {
    let _g = lock();
    let se = refusal("counter = 1\ncountr");
    assert!(
        se.did_you_mean.iter().any(|n| n == "counter"),
        "did_you_mean should offer the bound variable: {:?}",
        se.did_you_mean
    );
}

#[test]
fn an_unbound_name_inside_interpolation_is_an_error() {
    let _g = lock();
    let se = refusal("\"x ${nosuchname_zz} y\"");
    assert_eq!(se.code, ErrorCode::UnknownName);
}

#[test]
fn a_caught_unbound_name_carries_its_code() {
    let _g = lock();
    assert_eq!(
        run("try { nosuchname_zz } catch e { e.code }").unwrap(),
        Value::Str("E_UNKNOWN_NAME".to_string())
    );
}

#[test]
fn bound_names_still_evaluate() {
    let _g = lock();
    assert_eq!(run("counter = 41\ncounter + 1").unwrap(), Value::Int(42));
    assert_eq!(run("f = fn(x) => x + 1\nf(2)").unwrap(), Value::Int(3));
    assert_eq!(
        run("match 5 { s if s >= 1 => \"pos\", _ => \"other\" }").unwrap(),
        Value::Str("pos".to_string())
    );
    assert_eq!(run("null").unwrap(), Value::Null);
    assert_eq!(
        run("str.upper(\"ab\")").unwrap(),
        Value::Str("AB".to_string())
    );
}

#[test]
fn a_builtin_name_as_a_value_is_not_unknown() {
    let _g = lock();
    assert_eq!(run("g = len; g([1, 2, 3])").unwrap(), Value::Int(3));
}
