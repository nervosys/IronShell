//! `&&` and `||` short-circuit: the right operand runs only when the left one
//! does not decide the result. Reproduced on 14.0.1: `x = ""` then
//! `false && (x >= 1)` raised E_BAD_ARG comparing "" with 1, so the usual
//! guard `r.amount != "" && r.amount > 100` failed on the very row it guards.

use iron_shell::env::Env;
use iron_shell::eval::eval_program;
use iron_shell::parser::parse_program;
use iron_shell::value::Value;

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

#[test]
fn a_false_left_side_skips_the_right_of_and() {
    let v = eval("x = \"\"\nfalse && (x >= 1)").expect("no comparison runs");
    assert!(matches!(v, Value::Bool(false)), "{v:?}");
}

#[test]
fn a_true_left_side_skips_the_right_of_or() {
    let v = eval("x = \"\"\ntrue || (x >= 1)").expect("no comparison runs");
    assert!(matches!(v, Value::Bool(true)), "{v:?}");
}

#[test]
fn the_right_side_still_runs_when_it_decides() {
    assert!(eval("x = \"\"\ntrue && (x >= 1)").is_err());
    assert!(eval("x = \"\"\nfalse || (x >= 1)").is_err());
    assert!(matches!(eval("true && 2 > 1").unwrap(), Value::Bool(true)));
    assert!(matches!(eval("false || 2 > 1").unwrap(), Value::Bool(true)));
}

#[test]
fn a_guard_in_where_skips_the_empty_field() {
    let v = eval(
        "rows = [{a: \"\"}, {a: 5}, {a: 1}]\nrows | where(fn(r) => r.a != \"\" && r.a > 2) | len",
    )
    .expect("the guard protects the comparison");
    assert!(matches!(v, Value::Int(1)), "{v:?}");
}
