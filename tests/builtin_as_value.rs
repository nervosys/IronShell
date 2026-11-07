//! A builtin's name, bound to a variable, is a callable value.
//!
//! `g = rm` used to bind Null, because a bare name that nothing binds evaluated
//! to Null, and `g("x")` then failed with "expected a function in `g`, got
//! Null". A builtin name with no binding now evaluates to the builtin reference
//! a module member already is, so `g` calls the same builtin, through the same
//! gate. A name that is neither bound nor a builtin is still Null.

use iron_shell::env::Env;
use iron_shell::safety::ErrorCode;
use iron_shell::value::{BuiltinRef, Value};
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

#[test]
fn a_bound_builtin_name_is_a_builtin_value() {
    let _g = lock();
    assert_eq!(
        run("g = len; g").unwrap(),
        Value::Builtin(BuiltinRef {
            name: "len".to_string()
        })
    );
}

#[test]
fn a_bound_builtin_name_calls_the_builtin() {
    let _g = lock();
    assert_eq!(run("g = len; g([1, 2, 3])").unwrap(), Value::Int(3));
}

#[test]
fn an_unbound_name_that_is_not_a_builtin_is_still_null() {
    let _g = lock();
    assert_eq!(run("nosuchname_zz").unwrap(), Value::Null);
}

#[test]
fn a_user_binding_still_wins_over_the_builtin_name() {
    let _g = lock();
    assert_eq!(run("len = 5; len").unwrap(), Value::Int(5));
}

#[test]
fn a_bound_destructive_builtin_is_still_gated_in_agent_mode() {
    // The value is the same builtin, so the gate that answers the direct call
    // answers the call through the variable too.
    let _g = lock();
    let base = std::env::temp_dir().join(format!("ae_biv_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("c.txt"), "x").unwrap();
    std::env::set_var("IRONSHELL_MODE", "agent");
    std::env::set_var("IRONSHELL_WORKSPACE", &base);
    std::env::remove_var("IRONSHELL_APPROVE");

    let target = base.join("c.txt").to_string_lossy().replace('\\', "/");
    let r = run(&format!("g = rm; g(\"{target}\")"));

    std::env::remove_var("IRONSHELL_WORKSPACE");
    std::env::remove_var("IRONSHELL_MODE");
    let err = r.expect_err("a delete through a variable must need approval");
    let se = err
        .downcast_ref::<iron_shell::safety::SafetyError>()
        .expect("structured refusal");
    assert_eq!(se.code, ErrorCode::NeedsApproval);
    assert!(
        base.join("c.txt").exists(),
        "the refused delete must not run"
    );
    let _ = std::fs::remove_dir_all(&base);
}
