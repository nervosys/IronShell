//! A lambda's captured variables are visible when a builtin calls it.
//!
//! `map`, `where` and `reduce` call lambdas through their own fast paths, which
//! bound the parameters but not the captured free variables. A curried lambda,
//! `fn(n) => fn(x) => x * n`, worked when called directly and saw `n` unbound
//! when handed to `map`: null before unbound names were errors, and
//! E_UNKNOWN_NAME after.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn ints(v: &[i64]) -> Value {
    Value::Array(v.iter().map(|i| Value::Int(*i)).collect())
}

#[test]
fn a_curried_lambda_keeps_its_capture_under_map() {
    let v = run("multiply_by = fn(n) => fn(x) => x * n\n\
                 double = multiply_by(2)\n\
                 [1, 2, 3] | map(double)");
    assert_eq!(v, ints(&[2, 4, 6]));
}

#[test]
fn a_curried_lambda_keeps_its_capture_under_where() {
    let v = run("above = fn(n) => fn(x) => x > n\n\
                 [1, 5, 2, 7] | where(above(3))");
    assert_eq!(v, ints(&[5, 7]));
}

#[test]
fn a_two_parameter_curried_lambda_keeps_its_capture_under_reduce() {
    let v = run("scaled_sum = fn(k) => fn(a, b) => a + b * k\n\
                 [1, 2, 3] | reduce(scaled_sum(10), 0)");
    assert_eq!(v, Value::Int(60));
}

#[test]
fn a_parameter_still_shadows_a_captured_name() {
    let v = run("x = 100\n\
                 f = fn(n) => fn(x) => x + n\n\
                 [1, 2] | map(f(10))");
    assert_eq!(v, ints(&[11, 12]));
}

#[test]
fn the_capture_does_not_leak_after_the_call() {
    let v = run("mk = fn(secret_zz) => fn(x) => x + secret_zz\n\
                 [1] | map(mk(5))\n\
                 try { secret_zz } catch e { e.code }");
    assert_eq!(v, Value::Str("E_UNKNOWN_NAME".into()));
}
