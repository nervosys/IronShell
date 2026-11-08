//! `sort` refuses a list of Records, Bools, Arrays or nulls.
//!
//! Those have no order, and the comparator used to call every pair equal, so
//! the list came back in input order with exit 0. Each is now E_BAD_ARG, and
//! the hint names `sort_by(fn(r) => r.field)`, which is how a list of records
//! is sorted. Numbers and strings sort as before, and so does a record list
//! sorted by a field.

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
    let err = run(src).expect_err("an unordered list must be refused");
    err.downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

#[test]
fn a_list_of_records_is_refused_with_the_sort_by_hint() {
    let _g = lock();
    let se = refusal("[{a: 2}, {a: 1}] | sort");
    assert_eq!(se.code, ErrorCode::BadArg);
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert_eq!(se.message, "sort: expected numbers or strings, got Record");
    assert!(
        se.hint.contains("sort_by(fn(r) => r.field)"),
        "hint: {}",
        se.hint
    );
}

#[test]
fn a_list_of_bools_arrays_or_nulls_is_refused() {
    let _g = lock();
    assert_eq!(
        refusal("[true, false] | sort").message,
        "sort: expected numbers or strings, got Bool"
    );
    assert_eq!(
        refusal("[[2], [1]] | sort").message,
        "sort: expected numbers or strings, got Array"
    );
    assert_eq!(
        refusal("[null, 1] | sort").message,
        "sort: expected numbers or strings, got Null"
    );
}

#[test]
fn a_direct_call_refuses_the_same_way() {
    let _g = lock();
    assert_eq!(refusal("sort([{a: 1}])").code, ErrorCode::BadArg);
}

#[test]
fn numbers_and_strings_still_sort() {
    let _g = lock();
    assert_eq!(
        run("[3, 1, 2] | sort").unwrap(),
        Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_eq!(
        run("[\"b\", \"a\"] | sort").unwrap(),
        Value::Array(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string())
        ])
    );
}

#[test]
fn a_record_list_sorted_by_a_field_is_the_way_it_is_done() {
    let _g = lock();
    assert_eq!(
        run("[{a: 2}, {a: 1}] | sort_by(fn(r) => r.a) | map(fn(r) => r.a)").unwrap(),
        Value::Array(vec![Value::Int(1), Value::Int(2)])
    );
}
